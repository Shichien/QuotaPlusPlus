cask "qpp" do
  arch arm: "arm64", intel: "x64"

  version "0.3.2"
  sha256 :no_check

  url "https://github.com/Shichien/QuotaPlusPlus/releases/download/v#{version}/qpp-macos-#{arch}.dmg",
      verified: "github.com/Shichien/QuotaPlusPlus/"
  name "QuotaPlusPlus"
  desc "Switch Codex between official login and a custom Responses API provider"
  homepage "https://github.com/Shichien/QuotaPlusPlus"

  app "QuotaPlusPlus.app"
  binary "#{appdir}/QuotaPlusPlus.app/Contents/MacOS/qpp"

  caveats <<~EOS
    The current macOS build is unsigned. Install with --no-quarantine, or
    right-click QuotaPlusPlus in Finder and choose Open the first time.
  EOS
end
