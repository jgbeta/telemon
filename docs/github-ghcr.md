# GitHub And GHCR Publishing

Telemon publishes two public Docker images:

```text
ghcr.io/<owner>/telemon-hub
ghcr.io/<owner>/telemon-exporter
```

The GitHub repository should be named `telemon`, with `main` as the default
branch.

## Tags

Pushes to `main` publish:

```text
edge
sha-<short-sha>
```

Version tags publish:

```text
latest
X.Y.Z
X.Y
vX.Y.Z
sha-<short-sha>
```

Create a version release with:

```bash
git tag v0.1.0
git push origin v0.1.0
```

## First Publish

1. Create the GitHub repository named `telemon`.
2. Push the local `main` branch.
3. Open the `Docker Images` workflow and confirm both images published.
4. In GitHub Packages, set both packages to public if GitHub created them as
   private.
5. Pull the images from a target host:

```bash
docker pull ghcr.io/<owner>/telemon-hub:edge
docker pull ghcr.io/<owner>/telemon-exporter:edge
```

## Compose Usage

Set image variables in a `.env` file near the compose file or export them in the
shell:

```bash
export TELEMON_HUB_IMAGE=ghcr.io/<owner>/telemon-hub:edge
export TELEMON_EXPORTER_IMAGE=ghcr.io/<owner>/telemon-exporter:edge
```

Then run:

```bash
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/exporter/docker-compose.production.yml up -d
```

Development compose files still build locally from source.

