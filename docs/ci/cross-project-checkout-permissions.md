# Cross-project CI checkout permissions

Janus PR #74 runs read-only verification against pinned commits in the private
OmniVault-Android, Janus Community, and Multicolored Hat Society repositories.
The default `GITHUB_TOKEN` issued to a Janus workflow is scoped to Janus-OS and
cannot clone those private repositories. GitHub reports this as
`Repository not found` during the external checkout step.

To enable the cross-project jobs, a repository administrator must create a
fine-grained personal access token (or equivalent GitHub App installation token)
with **Contents: Read-only** access to these repositories:

- `Wizardwalt/OmniVault-Android`
- `Wizardwalt/janus-community`
- `Wizardwalt/multicolored-hat-society`

Store it as the Actions repository secret `CROSS_PROJECT_READ_TOKEN` in
`Wizardwalt/Janus-OS`. Do not commit the token, place it in workflow files, or
print it in logs. The cross-project checkout steps prefer this secret and fall
back to the repository-scoped `github.token`; the fallback will continue to
fail for private repositories until the secret is configured.

After adding the secret, rerun the failed `cross-project-android` and
`cross-project-backend` jobs on PR #74. The physical-device OmniVault → Janus
→ encrypted Aria history round-trip remains a separate acceptance test.
