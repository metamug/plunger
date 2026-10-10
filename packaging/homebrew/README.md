# Homebrew

`plunger.rb` is the cask for Plunger. A cask lives in a *tap*, a GitHub repository named `homebrew-<name>`.

## One-time setup

1. Create the repository `metamug/homebrew-tap` (public).
2. Add `Casks/plunger.rb` from this folder, with the real `version` and `sha256` of the latest
   `Plunger-<version>-macos.dmg` on the release page (the `.sha256` file next to it has the value).
3. People can then install with:

   ```bash
   brew install --cask metamug/tap/plunger
   ```

   which puts `Plunger.app` in `/Applications` and a `plunger` command on the PATH.

## Each release

Change `version` and `sha256` and push. (A small workflow in the tap repository can do this on each release.)

## Check it before publishing

```bash
brew audit --new --cask ./plunger.rb
brew install --cask ./plunger.rb
```

## The first launch

The app is signed ad hoc, not with an Apple Developer ID, so macOS asks the person to approve it once
(System Settings > Privacy & Security > Open Anyway, or right-click the app and choose Open). Homebrew
users can skip that with `brew install --cask --no-quarantine metamug/tap/plunger`.
