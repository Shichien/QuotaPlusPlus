cask "qpp" do
  arch arm: "arm64", intel: "x64"

  version "0.1.3"
  sha256 :no_check

  url "https://github.com/Shichien/QuotaPlusPlus/releases/download/v#{version}/qpp-macos-#{arch}.dmg",
      verified: "github.com/Shichien/QuotaPlusPlus/"
  name "QuotaPlusPlus"
  desc "Switch Codex between official login and a custom Responses API provider"
  homepage "https://github.com/Shichien/QuotaPlusPlus"

  app "QuotaPlusPlus.app"
  binary "#{appdir}/QuotaPlusPlus.app/Contents/MacOS/qpp"

  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/QuotaPlusPlus.app"]
  end
end
