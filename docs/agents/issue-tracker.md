# Issue tracker: GitHub

Issues and specs for Neptune Finder live in the GitHub repository configured
as this checkout's `origin`. Use the `gh` CLI. Do not send Finder issues or PRs
to the separate Neptune Terminal repository.

Run commands inside this checkout, where `gh` infers the repository from `origin`.
Resolve its owner and name with `gh repo view --json nameWithOwner --jq .nameWithOwner`.
Outside the checkout, pass that value with `--repo` to `gh issue` and `gh pr`
commands and use `repos/<owner>/<repo>` for REST API paths.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body-file <path>`.
- **Read an issue**: `gh issue view <number> --comments`. For structured metadata and labels, use `--json number,title,body,labels,comments,assignees,state`.
- **List issues**: `gh issue list --state open --limit 100 --json number,title,body,labels,comments,assignees`, with appropriate `--label` filters. Paginate with `gh api --paginate` when a complete inventory is required.
- **Comment**: `gh issue comment <number> --body-file <path>`.
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`. Use the mapping in [triage-labels.md](triage-labels.md).
- **Close**: `gh issue close <number>`. If an explanation is needed, post it first using `gh issue comment --body-file`.

For multiline issue bodies, PR bodies and comments, write the exact Markdown to a temporary file and pass it with `--body-file`.

## Pull request delivery

For every change, including hotfixes:

1. Before committing, check `git branch --show-current`. If on `main` or another protected branch, create or switch to a feature branch first. Start new branches from `main` and preserve unrelated work.
2. Push only the feature branch with `git push --set-upstream origin <branch>`, then open a PR with `gh pr create --base main --head <branch> --title "..." --body-file <path>`.
3. Check CI for the latest PR head with `gh pr checks <number> --watch --fail-fast`. CI must be green before merge; pending, failed or cancelled checks block it. Repeat this check after any new commits.
4. Merge through the PR with `gh pr merge <number>` only after CI is green and branch protection requirements are satisfied. Do not use `--admin` to bypass these gates.

## PR attachments

Use the user's browser to attach screenshots, videos and other evidence files
to a PR.

1. Open the PR description editor, or the creation form when creating a PR was
   requested. Preserve the existing description.
2. Use GitHub's attachment control with the browser's local-file upload capability.
   Transfer files directly; keep binary/base64 data out of tool output.
3. Wait for uploads to finish, retain GitHub's generated attachment links in the
   evidence section, and save the description.
4. Reopen the rendered description and verify that images display and file links
   open.

If GitHub requires sign-in, ask the user to sign in in that browser and continue
there. If the initial browser upload fails or its tools cannot upload local
files, retry the attachment workflow in the user's personal Chrome browser when
available. If neither browser can complete the upload, report the blocker and
provide the exact file paths for the user to attach.

## Pull requests as a triage surface

Use issues for requests and specs; use PRs to deliver changes.

GitHub issues and PRs share a number space. Resolve an ambiguous number with `gh api repos/<owner>/<repo>/issues/<number>`: a `pull_request` field identifies a PR. For an explicitly named PR, use `gh pr view <number> --comments` and `gh pr diff <number>`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The map is a single issue with child issues as tickets.

- **Map**: an issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. Create it with `gh issue create --title "..." --label wayfinder:map --body-file <path>`.
- **Child ticket**: create an issue and link it as a GitHub sub-issue with `gh api --method POST repos/<owner>/<repo>/issues/<map>/sub_issues -F sub_issue_id=<child-db-id>`. Obtain the numeric database id with `gh api repos/<owner>/<repo>/issues/<child> --jq .id`. Where sub-issues are unavailable, add the child to an ordered task list in the map and put `Part of #<map>` at the top of the child body. Labels are `wayfinder:<type>` (`research`, `prototype`, `grilling`, or `task`).
- **Blocking**: use native issue dependencies with `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`. Obtain the blocker's database id with `gh api repos/<owner>/<repo>/issues/<blocker> --jq .id`; use the database id rather than the issue number or node id. If dependencies are unavailable, record `Blocked by: #<number>, #<number>` at the top of the child body. A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's children with `gh api --paginate repos/<owner>/<repo>/issues/<map>/sub_issues`, or read the fallback task list. Keep open, unassigned children. For each child, list blockers with `gh api --paginate repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by`, or read the fallback `Blocked by` line, and exclude children with an open blocker. The first remaining child in map order wins.
- **Claim**: `gh issue edit <number> --add-assignee @me`, the session's first write.
- **Resolve**: comment with `gh issue comment <number> --body-file <path>`, close the child, then append a context pointer (gist + link) to the map's Decisions-so-far using `gh issue edit <map> --body-file <path>`.

For endpoint details, see GitHub's [sub-issues](https://docs.github.com/en/rest/issues/sub-issues) and [issue dependencies](https://docs.github.com/en/rest/issues/issue-dependencies) references.
