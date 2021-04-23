use git2::Repository;
use git_url_parse::GitUrl;
use hubcaps::deployments::{DeploymentListOptions, DeploymentOptions};
use hubcaps::{statuses, Credentials, Github};
use indicatif::ProgressBar;
use std::{env, thread, time};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "deploy", about = "A CLI tool to trigger GitHub deployments")]
struct Opt {
    #[structopt(short = "r", long = "ref", aliases = &["branch", "commit", "tag"], help = "The git ref to deploy. Can be a git commit, branch, or tag.")]
    git_ref: Option<String>,

    #[structopt(short, long, help = "The environment to deploy to")]
    env: Option<String>,

    // TODO(mmk) Need to hook this up still
    #[structopt(short, long, help = "Ignore commit status checks")]
    force: bool,

    #[structopt(short, long, help = "Don't wait for the deployment to complete")]
    detached: bool,

    #[structopt(short, long, help = "Silence any output to STDOUT")]
    quiet: bool,

    #[structopt(help = "The repository you want to deploy. Defaults to current repository")]
    repository: Option<String>,
}

fn parse_owner_and_name_from_remote_url(
    url: String,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let git_url = GitUrl::parse(&url)?;
    let owner = git_url.owner;

    match git_url.host {
        Some(host) if host == "github.com" && owner.is_some() => Ok((owner.unwrap(), git_url.name)),
        _ => Err("Host could not be determined or is not a GitHub remote".into()),
    }
}

fn determine_repository_string(
    repository: Option<String>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Some(r) = repository {
        return parse_owner_and_name_from_remote_url(format!("https://github.com/{}", r));
    }

    // TODO(mmk) Under which conditions does this fail?
    let repository = Repository::open(env::current_dir()?)?;
    let remote = repository.find_remote("origin")?;

    match remote.url() {
        Some(url) => parse_owner_and_name_from_remote_url(url.into()),
        None => Err("No URL set for origin remote".into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match env::var("GITHUB_TOKEN").ok() {
        Some(token) => {
            let opt = Opt::from_args();

            let github = Github::new(
                concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
                Credentials::Token(token),
            )?;

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
            let results = deployments.list(list_options).await?;

            if !results.is_empty() {
                for d in results {
                    let sha = d.sha;
                    spinner.println(format!(
                        "See commit difference at https://github.com/{}/{}/compare/{}...{}",
                        &owner,
                        &repository,
                        // TODO(mmk) We cannot assume this has been set.
                        opt.git_ref.clone().unwrap(),
                        sha
                    ));
                    break;
                }
            }

            let options = &DeploymentOptions::builder(opt.git_ref.clone().unwrap())
                .environment(opt.env.clone().unwrap())
                // TODO(mmk) We need a better description to be provided here.
                .description::<String>(
                    "A practice deployment from the rust version of deploy".into(),
                )
                .build();
            spinner.set_message("Triggering deployment");
            let deploy = deployments.create(options).await?;

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

            loop {
                thread::sleep(time::Duration::from_millis(300));
                let statuses = deployments.statuses(deploy.id).list().await?;

                if statuses.is_empty() {
                    spinner.set_message("Waiting for deployments to begin");
                    continue;
                }

                let status = statuses.first().unwrap();
                match status.state {
                    statuses::State::Pending => {
                        spinner.set_message("Building");
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
        _ => Err("Missing GITHUB_TOKEN. Please set this environment variable.".into()),
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
