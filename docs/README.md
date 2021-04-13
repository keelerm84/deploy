# deploy

A small rust program for creating [GitHub Deployments][github-deployments].
Lovingly ported from [the original Go code][ported-from-go].

## Usage

You must generate a GitHub Personal Access Token and save it in the environment
variable `GITHUB_TOKEN`.

Deploy the main branch of a repo to staging:

```console
$ deploy --ref=main --env=staging keelerm84/deploy
```

Deploy the main branch of the current repo to staging:

```console
$ deploy --ref=main --env=staging
```

[github-deployments]: https://developer.github.com/v3/repos/deployments/
[ported-from-go]: https://github.com/remind101/deploy
