use anyhow::{anyhow, Context, Result};
use git2::Repository;
use git_url_parse::GitUrl;
use hubcaps::deployments::{DeploymentListOptions, DeploymentOptions, DeploymentStatus};
use hubcaps::{statuses, Credentials, Github};
use indicatif::ProgressBar;
use std::{env, thread, time};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "deploy")]
/// A CLI tool to trigger GitHub deployments
struct Opt {
    /// The git ref to deploy. Can be a git commit, branch, or tag. When <repository> is specified, <git-ref> is also required.
    #[structopt(short = "r", long = "ref", aliases = &["branch", "commit", "tag"])]
    git_ref: Option<String>,

    /// The environment to deploy to
    #[structopt(short, long)]
    env: Option<String>,

    /// Ignore commit status checks
    #[structopt(short, long)]
    force: bool,

    /// Don't wait for the deployment to complete
    #[structopt(short, long)]
    detached: bool,

    /// Silence any output to STDOUT
    #[structopt(short, long)]
    quiet: bool,

    /// The repository you want to deploy to. Defaults to current repository
    #[structopt(requires = "git-ref")]
    repository: Option<String>,

    #[structopt(subcommand)]
    cmd: Option<Command>,
}

#[derive(Debug, PartialEq, StructOpt)]
enum Command {
    Update,
}

fn parse_owner_and_name_from_remote_url(url: String) -> Result<(String, String)> {
    let git_url = GitUrl::parse(&url)?;
    let owner = git_url.owner;

    match git_url.host {
        Some(host) if host == "github.com" && owner.is_some() => Ok((owner.unwrap(), git_url.name)),
        _ => Err(anyhow!(
            "Host could not be determined or is not a GitHub remote"
        )),
    }
}

fn determine_repository_string(repository: Option<String>) -> Result<(String, String)> {
    if let Some(r) = repository {
        return parse_owner_and_name_from_remote_url(format!("https://github.com/{}", r));
    }

    // TODO(mmk) Under which conditions does this fail?
    let current_dir = env::current_dir().context("Unable to get cwd")?;
    let repository = Repository::open(current_dir).context("Cannot access local repository")?;
    let remote = repository.find_remote("origin")?;
    let url = remote.url().context("No URL set for origin")?;

    Ok(parse_owner_and_name_from_remote_url(url.into())?)
}

fn determine_current_branch() -> Result<String> {
    let repository = Repository::open(env::current_dir()?)?;
    let head = repository
        .head()
        .context("Unable to determine HEAD of repository")?;
    let name = head.name().context("Unable to determine current branch")?;

    Ok(name.into())
}

#[tokio::main]
async fn main() -> Result<()> {
    match env::var("GITHUB_TOKEN").ok() {
        Some(token) => {
            let opt = Opt::from_args();

            if let Some(cmd) = opt.cmd {
                return match cmd {
                    Command::Update => {
                        let status = self_update::backends::github::Update::configure()
                            .repo_owner("keelerm84")
                            .repo_name(env!("CARGO_PKG_NAME"))
                            .bin_name("deploy")
                            .show_download_progress(true)
                            .current_version(env!("CARGO_PKG_VERSION"))
                            .build()?
                            .update()?;
                        println!("Update status: `{}`!", status.version());
                        Ok(())
                    }
                };
            }

            let github = Github::new(
                concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
                Credentials::Token(token),
            )
            .context("Unable to create Github client.")?;

            let spinner = match opt.quiet {
                true => ProgressBar::hidden(),
                false => ProgressBar::new_spinner(),
            };
            spinner.set_style(
                indicatif::ProgressStyle::default_spinner().template("{spinner:.green} {msg}"),
            );
            spinner.enable_steady_tick(100);

            let (owner, repository) = determine_repository_string(opt.repository)?;
            let repo = github.repo(&owner, &repository);
            let deployments = repo.deployments();
            let list_options = &DeploymentListOptions::builder()
                .environment(opt.env.clone().unwrap())
                .build();

            // TODO(mmk) What is the ordering here? Can we always assume the first one is the most
            // recent, or do we need to sort?
            //
            // How should we handle failed deployments? Pending deployments?
            let results = deployments
                .list(list_options)
                .await
                .context("Unable to get a list of deployments")?;

            let git_ref = match opt.git_ref {
                Some(reference) => reference,
                None => determine_current_branch()?,
            };

            if !results.is_empty() {
                for d in results {
                    let sha = d.sha;
                    spinner.println(format!(
                        "See commit difference at https://github.com/{}/{}/compare/{}...{}",
                        &owner, &repository, git_ref, sha
                    ));
                    break;
                }
            }

            let mut builder = DeploymentOptions::builder(git_ref);
            builder
                .auto_merge(false)
                .environment(opt.env.clone().unwrap())
                // TODO(mmk) We need a better description to be provided here.
                .description::<String>(
                    "A practice deployment from the rust version of deploy".into(),
                );

            // From the GitHub deployment API documentation:
            //
            // The status contexts to verify against commit status checks. If you omit this
            // parameter, GitHub verifies all unique contexts before creating a deployment. To
            // bypass checking entirely, pass an empty array. Defaults to all unique contexts.
            if opt.force == true {
                let contexts: Vec<String> = Vec::new();
                builder.required_contexts(contexts);
            }

            spinner.set_message("Triggering deployment");
            let deploy = deployments
                .create(&builder.build())
                .await
                .context("Could not create the specified deployment")?;

            spinner.set_style(
                indicatif::ProgressStyle::default_spinner().template(&format!(
                    "{{spinner:.green}} [{}:{}] {{msg}}",
                    deploy.environment,
                    deploy.id.to_string()
                )),
            );

            if opt.detached {
                return Ok(());
            }

            let mut failures: i32 = 0;
            loop {
                thread::sleep(time::Duration::from_millis(300));
                let statuses: Vec<DeploymentStatus>;
                if let Ok(s) = deployments.statuses(deploy.id).list().await {
                    statuses = s;
                    failures = 0;
                } else if failures == 3 {
                    return Err(anyhow!("Failed to check deployment status. Exiting."));
                } else {
                    failures += 1;
                    continue;
                }

                if statuses.is_empty() {
                    spinner.set_message("Waiting for deployments to begin");
                    continue;
                }

                let status = statuses
                    .first()
                    .context("Could not read first deployment status")?;

                match status.state {
                    statuses::State::Pending => {
                        spinner.set_message("Deploying");
                        continue;
                    }
                    statuses::State::Error => {
                        spinner.finish_with_message(&format!(
                            "Build finished with error. {}",
                            status
                                .description
                                .clone()
                                .unwrap_or_else(|| "No description given".into())
                        ));
                    }
                    statuses::State::Success => {
                        spinner.finish_with_message("Done!");
                    }
                    statuses::State::Failure => {
                        spinner.finish_with_message(&format!(
                            "Build finished with error. {}",
                            status
                                .description
                                .clone()
                                .unwrap_or_else(|| "No description given".into())
                        ));
                    }
                }

                break;
            }

            Ok(())
        }
        _ => Err(anyhow!(
            "Missing GITHUB_TOKEN. Please set this environment variable."
        )),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use test_case::test_case;

    #[test_case("git@github.com:keelerm84/deploy.git", "keelerm84", "deploy"; "ssh style")]
    #[test_case("git@github.com:keelerm84/deploy", "keelerm84", "deploy"; "ssh style without .git")]
    #[test_case("https://github.com/keelerm84/deploy.git", "keelerm84", "deploy"; "https style")]
    #[test_case("https://github.com/keelerm84/deploy", "keelerm84", "deploy"; "https style without .git")]
    fn test_correctly_parses_github_remote(url: &str, owner: &str, repo: &str) {
        match parse_owner_and_name_from_remote_url(url.to_string()) {
            Ok((o, r)) => {
                assert_eq!(owner, o);
                assert_eq!(repo, r);
            }
            _ => assert!(false, format!("{} should not generate an error", url)),
        }
    }

    #[test_case("git@bitbucket.com:keelerm84/deploy.git"; "ssh style")]
    #[test_case("https://bitbucket.com/keelerm84/deploy.git"; "http style")]
    fn test_cannot_parse_non_github_remote(url: &str) {
        assert!(
            parse_owner_and_name_from_remote_url(url.to_string()).is_err(),
            format!("{} should not be parsable", url)
        );
    }
}
