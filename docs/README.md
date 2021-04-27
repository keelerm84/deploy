# deploy

[![Trust but verify](https://github.com/keelerm84/deploy/actions/workflows/trust-but-verify.yml/badge.svg)](https://github.com/keelerm84/deploy/actions/workflows/trust-but-verify.yml)

A small rust program for creating [GitHub Deployments][github-deployments].

## Environment Variables

To run this project, you will need to have the following environment variables set.

* `GITHUB_TOKEN`

   This [personal access token][tokens] is used to create deployments on any target repositories. \
   The token requires `repo:status` and `repo_deployment` access.

## Usage

Deploy the main branch of a repo to staging:

```console
$ deploy --ref=main --env=staging keelerm84/deploy
```

Deploy the main branch of the current repo to staging:

```console
$ deploy --ref=main --env=staging
```

Deploy the current branch of the current repo to staging:

```console
$ deploy --env=staging
```

Update the executable from the latest GitHub release:

```console
$ deploy update
```

## License

[MIT](./LICENSE.md)

## Acknowledgment

This project is a Rust implementation from an existing go project. You can view
the original project [here][ported-from-go].

[github-deployments]: https://developer.github.com/v3/repos/deployments/
[tokens]: https://github.com/settings/tokens
[ported-from-go]: https://github.com/remind101/deploy
