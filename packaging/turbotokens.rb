class Turbotokens < Formula
  desc "Real-time token and cost telemetry for AI coding agents"
  homepage "https://github.com/maxmoneycash/turbotokens"
  version "1.1.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-arm64.tar.gz"
      sha256 "0f2ad64c7a0b5ed4351a4ad83afe0ea9216d69118b80f3a9d555d71081e26293"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-x64.tar.gz"
      sha256 "36f4e049ee51d231ae253e1139da157f04130aca7c374ac4790f9921ac5a0ede"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-arm64.tar.gz"
      sha256 "8bf7ec90c3366293010eddcc46f0eeb656970b4b5a4d8c2e4a8c09d1b498a768"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-x64.tar.gz"
      sha256 "17cc06aa514786e295777a49c6c21730954d2583070b09224761fd16d774ecec"
    end
  end

  def install
    bin.install "turbotokens"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/turbotokens --version")
  end
end
