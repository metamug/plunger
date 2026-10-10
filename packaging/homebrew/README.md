# Homebrew

`plunger.rb` is the cask for Plunger. A cask lives in a *tap*, a GitHub repository named `homebrew-<name>`.

## The tap

The tap is live at [metamug/homebrew-tap](https://github.com/metamug/homebrew-tap):

```bash
brew install --cask metamug/tap/plunger
```

which puts `Plunger.app` in `/Applications` and a `plunger` command on the PATH. Its workflow installs the
cask on a macOS runner on every push (and weekly) and runs `plunger --version`.

## Each release

Change `version` and `sha256` in the tap's `Casks/plunger.rb` (the `.sha256` file next to the `.dmg` on the
release page has the value) and push; keep `packaging/homebrew/plunger.rb` here in step, as the template.

## Check it before publishing

```bash
brew audit --new --cask ./plunger.rb
brew install --cask ./plunger.rb
```

## The first launch

The app is signed ad hoc, not with an Apple Developer ID, so macOS asks the person to approve it once
(System Settings > Privacy & Security > Open Anyway, or right-click the app and choose Open). Homebrew
users can skip that with `brew install --cask --no-quarantine metamug/tap/plunger`.
