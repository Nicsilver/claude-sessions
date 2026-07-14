# Windows code signing via SignPath Foundation

Status: **prep only.** We ship unsigned until the SignPath Foundation application is
approved. This file holds everything needed to apply and to switch signing on.

## When to apply

SignPath Foundation weights the **Reputation** field heavily and favours established,
widely-used projects. Apply once `claude-sessions` has some traction to point at:
GitHub stars, release download counts (see the Releases page insights), and any
blog / Reddit / Hacker News mentions. Applying with an empty reputation story is
likely to be declined.

Prerequisites the application checks:
- OSI-approved license — done (AGPL-3.0).
- Project already released — done (v0.1.0 onward).
- The **Download URL page must state** the project is signed via SignPath Foundation.
  Add the credit line below to the release notes / README once approved.

## Application answers (signpath.org/apply)

| Field | Value |
|---|---|
| Project Name | Claude Sessions |
| Repository URL | https://github.com/Nicsilver/claude-sessions |
| Homepage URL | https://github.com/Nicsilver/claude-sessions |
| Download URL | https://github.com/Nicsilver/claude-sessions/releases |
| Tagline | See at a glance which of your Claude Code sessions are waiting on you — across Windows, macOS, and JetBrains IDEs. |
| Maintainer Type | Individual |
| Build System | GitHub Actions |
| First / Last name, Email | (yours) |

**Description**

> claude-sessions surfaces the live status of every Claude Code session you have
> running. Claude Code lifecycle hooks record each session's state to a small JSON
> file; native surfaces then display it — a Windows system-tray widget, a macOS
> menu-bar app plus floating dashboard, and an IntelliJ/JetBrains plugin. A
> colour-coded badge and count show which sessions are working, which are your turn,
> and which need you, and a click jumps straight to the relevant terminal tab.

**Reputation** — fill at apply-time with: GitHub star count, total release download
count, and links to any external mentions (blog posts, Reddit, HN, etc.).

Note: the form has a reCAPTCHA and two required consent checkboxes (Code of Conduct
+ personal-data processing) — complete those yourself at submit time.

## Enabling signing after approval

On approval, SignPath provisions an **organization** and a **signing policy**, and you
create a **CI API token**. Add these repo secrets (Settings → Secrets and variables →
Actions):

- `SIGNPATH_API_TOKEN` — the CI user API token
- `SIGNPATH_ORGANIZATION_ID` — the organization GUID

Then insert this step into the `windows` job in `.github/workflows/release.yml`, **after**
the `Stage package` step uploads the artifact (SignPath pulls the artifact from the run,
signs it, and returns the signed zip):

```yaml
      # Requires: repo approved by SignPath Foundation; secrets set (see docs/signing.md)
      - name: Sign with SignPath
        uses: signpath/github-action-submit-signing-request@v1
        with:
          api-token: ${{ secrets.SIGNPATH_API_TOKEN }}
          organization-id: ${{ secrets.SIGNPATH_ORGANIZATION_ID }}
          project-slug: claude-sessions
          signing-policy-slug: release-signing        # or 'test-signing' while validating
          github-artifact-id: ${{ steps.upload-windows.outputs.artifact-id }}
          wait-for-completion: true
          output-artifact-directory: signed
```

Give the `upload-artifact` step `id: upload-windows` so `github-artifact-id` resolves,
and point the `release` job at the signed zip. SignPath signs the **contents** of the
zip (the `.exe`), so no other job changes are needed.

## Reference

- SignPath Foundation: https://signpath.org/  ·  terms: https://signpath.org/terms.html
- GitHub Action: https://github.com/signpath/github-action-submit-signing-request
- Signing is OV-level: SmartScreen reputation still builds over downloads, but the exe
  shows a verified publisher and reputation accrues to the stable certificate.
