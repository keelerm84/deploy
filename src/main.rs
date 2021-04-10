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

    // TODO(mmk) Need to hook this up still
    #[structopt(help = "The repository you want to deploy. Defaults to current repository")]
    repository: Option<String>,
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

            // TODO(mmk) Need to determine this pairing differently.
            let repo = github.repo("researchsquare", "deploy-me");
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
                        "researchsquare",
                        "deploy-me",
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
