use git2::Repository;
use hubcaps::deployments::{DeploymentListOptions, DeploymentOptions};
use hubcaps::{statuses, Credentials, Github};
use indicatif::ProgressBar;
use regex::Regex;
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

fn split_into_owner_and_repo(path: String) -> Result<(String, String), Box<dyn std::error::Error>> {
    let splits = path.split("/").collect::<Vec<&str>>();
    match splits.len() {
        2 => Ok((splits[0].into(), splits[1].into())),
        _ => Err("Provided repository does not match owner/repository format".into()),
    }
}

fn get_repository_and_owner(
    repository: Option<String>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    match repository {
        Some(r) => split_into_owner_and_repo(r),
        None => {
            let repository = Repository::open(env::current_dir()?)?;
            let remote = repository.find_remote("origin")?;
            match remote.url() {
                Some(url) => {
                    let re = Regex::new(r".*github.com.(?P<o>[^/]+)/(?P<r>.*).git")?;
                    let url = re.replace_all(url, "$o/$r");
                    split_into_owner_and_repo(url.into())
                }
                None => Err("No URL set for origin remote".into()),
            }
        }
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

            let (owner, repository) = get_repository_and_owner(opt.repository)?;
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
                                .unwrap_or_else(|| "No description given".to_string())
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
                                .unwrap_or_else(|| "No description given".to_string())
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
