class Turbotokens < Formula
  desc "Real-time token and cost telemetry for AI coding agents"
  homepage "https://github.com/maxmoneycash/turbotokens"
  version "1.1.4"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-arm64.tar.gz"
      sha256 "303560ef862c6d492ebd36b53f8e1330ca006ae3b9833a569bc9c40c6bff6bde"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-x64.tar.gz"
      sha256 "f9032335e8a5b886fd8af0db2864ad00abbc0db1a50a77d7d4e361812b29d759"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-arm64.tar.gz"
      sha256 "414ff26afb612af0ec639cc4901343ccfbe1a97a63c63d0179ded0e74fc05078"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-x64.tar.gz"
      sha256 "b4039010562d979a15c19af7b4331baf82a2e1bc2e458e7ba4a449d04d220345"
    end
  end

  def install
    bin.install "turbotokens"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/turbotokens --version")
  end
end
