# Desktop releases

Neptune Finder follows Neptune Terminal's deliberate SemVer and release-candidate
policy. Finder does not inherit Terminal's signing identities, updater keys,
download website, repository permissions or native acceptance evidence.

## Release-candidate policy

Every stable release, including hotfixes, requires accepted `X.Y.Z-rc.N`
installers first. Acceptance fixes increment `N`, keeping `X.Y.Z` fixed.
Releases are deliberate SemVer tags; a merge to `main` never releases software.
Prepared releases remain private drafts until reviewed and accepted.

**AI agents must not create/push release tags or create/publish Neptune Finder
releases unless explicitly asked to release that version.**

## Required release infrastructure

Before distributing a production release, provision and verify:

- Linux packages and desktop integration for the intended distributions.
- macOS Apple Silicon/Intel packaging, application identity, signing and notarization.
- Windows installer, application identity, signing and uninstall behavior.
- Reproducible checksums and retained dependency/font/icon license notices.
- A changelog and a documented, authenticated update/distribution process.
- Native acceptance on every promised OS and Linux display protocol.

The repository currently provides validation CI. It deliberately does not copy
a Terminal release workflow that would publish Finder with Terminal identifiers,
secrets or download locations. Packaging automation should be added only with
Finder's actual product identities and credentials, then tested without creating
production tags. Review [verification](verification.md) before promotion.

## Acceptance record

For each candidate retain its commit, lockfile, toolchain, package checksums,
runner platform, signing verification, native interaction results, screenshots
and measured resource/performance behavior. Resolve every remaining production
release gate before labeling a build production-ready. Build success and a
single Linux screenshot cannot substitute for native filesystem operations,
clipboard, drag/drop, IME and accessibility checks on the promised platforms.
