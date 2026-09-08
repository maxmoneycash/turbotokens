class Turbotokens < Formula
  desc "Real-time token and cost telemetry for AI coding agents"
  homepage "https://github.com/maxmoneycash/turbotokens"
  version "1.1.3"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-arm64.tar.gz"
      sha256 "e0ebdb307c70edb437ea23e98c2579b6747eefe03726174ce11f69f779768659"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-x64.tar.gz"
      sha256 "517cd526ee149e00e1ca6d29069cac0414e868f3202070b11c1717c619bbc811"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-arm64.tar.gz"
      sha256 "7256ba213586a8a37512b3dee112a63c868accd4af9166883ced8c74f4b924fe"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-x64.tar.gz"
      sha256 "3e024cbbd916e037d5073149a92843960e2871ab8a5d5270ce90d593235f25a3"
    end
  end

  def install
    bin.install "turbotokens"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/turbotokens --version")
  end
end
