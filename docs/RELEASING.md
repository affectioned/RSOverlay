# Releasing RSOverlay

There are two channels:

| | GitHub build | Steam build |
|---|---|---|
| Built by | `.github/workflows/release.yml`, job `github` | same workflow, job `steam` (off for now) |
| Cargo features | default | `--features steam` |
| Updates | In-app notice (Settings › Updates). The user decides; nothing installs itself. | Steam updates it automatically. No in-app notice. |
| Testers | Pre-releases (`v0.2.0-beta.1`), opt-in via "Include test builds" | A Steam beta branch (default `testers`) |

## Cutting a GitHub release

1. Set the version in the root `Cargo.toml` (`[workspace.package] version`).
2. Write the release notes in `docs/releases/<tag>.md` (e.g.
   `docs/releases/v0.2.0-beta.1.md`), for testers and users, not a commit
   list: install steps, what's new, known issues, how to report problems
   (no "please test" checklists: testers report what they find).
   **Visual changes get screenshots**: render them (`--vr-preview --scene …
   --screenshot`, `--settings-cards`, see `docs/store/README.md`), commit
   them (refresh `docs/store/` or add `docs/releases/<tag>/`), and link them
   with the URL pinned to the tag so the release keeps its own pictures:
   `![Caption](https://raw.githubusercontent.com/affectioned/RSOverlay/<tag>/docs/store/<file>.png)`.
   The in-app "What's new" skips image lines. Copy the
   shape of an earlier one. Commit both.
3. Tag and push:

   ```sh
   # tester build (GitHub pre-release; only users with "Include test builds" are told)
   git tag v0.2.0-beta.1
   git push origin v0.2.0-beta.1

   # everyone
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. The workflow tests, builds `--release`, zips `rsoverlay.exe` with the
   README, licences and the custom sounds guide, and publishes the release
   with your notes (GitHub's generated list if the file is missing) and a
   SHA-256 file.

The tag must match the Cargo version (a `-beta.N` / `-rc.N` suffix is
allowed), otherwise the workflow stops before building.

### How the update notice works

- At startup and every 6 hours, RSOverlay asks
  `api.github.com/repos/affectioned/RSOverlay/releases` (one anonymous GET,
  generic User-Agent, nothing about the user is sent).
- If a newer version exists, it shows one notification and the Updates card
  offers **Get update** (opens the release page), **Skip this version**, and
  **Check now**.
- Drafts are ignored. Pre-releases are only offered with **Include test
  builds** on. Links that don't point at this repository's releases are
  ignored.

## Turning on the Steam upload (later)

Once the Steamworks app exists:

1. In Steamworks, create the app's depot and a beta branch (e.g. `testers`,
   password-protected for testers).
2. Create a dedicated Steam **build account** with only the "Edit App
   Metadata" / "Publish App Changes To Steam" permissions for this app.
3. Generate its `config.vdf` once (log in with `steamcmd` and approve
   Steam Guard), then base64-encode it.
4. In GitHub, go to repository Settings › Secrets and variables › Actions:
   - Secrets: `STEAM_USERNAME`, `STEAM_CONFIG_VDF` (the base64 text)
   - Variables: `STEAM_APP_ID`, `STEAM_DEPOT_ID`, `STEAM_BETA_BRANCH`
     (optional, default `testers`), and `STEAM_ENABLED=true`
5. Every pushed tag now also builds with `--features steam` and uploads to
   the beta branch. Making a build live for everyone stays a manual step
   in Steamworks (Builds › set the build live on `default`).

Achievements are drafted in [ACHIEVEMENTS.md](ACHIEVEMENTS.md). They need
the Steamworks SDK and App ID before any code is written.
