# Homebrew cask for Plunger. To publish it, put this file at Casks/plunger.rb in a tap repository
# (metamug/homebrew-tap), with `version` and `sha256` set from a release's Plunger-<version>-macos.dmg
# and its .sha256 file. See packaging/homebrew/README.md.
cask "plunger" do
  version "0.7.4"
  sha256 "7a158a9a5b1fdd988249693196e08cbd13cdaa8821b27596f8d23c7fb7db607f"

  url "https://github.com/metamug/plunger/releases/download/v#{version}/Plunger-#{version}-macos.dmg"
  name "Plunger"
  desc "API client for you and your AI agent: window, command line and MCP server"
  homepage "https://github.com/metamug/plunger"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Plunger.app"
  # `plunger send ...` and `plunger mcp` from a terminal, using the app's own program.
  binary "#{appdir}/Plunger.app/Contents/MacOS/plunger"

  zap trash: [
    "~/Library/Application Support/Plunger",
    "~/Library/Saved Application State/com.metamug.plunger.savedState",
  ]
end
