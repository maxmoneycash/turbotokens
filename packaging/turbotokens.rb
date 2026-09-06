class Turbotokens < Formula
  desc "Real-time token and cost telemetry for AI coding agents"
  homepage "https://github.com/maxmoneycash/turbotokens"
  version "1.1.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-arm64.tar.gz"
      sha256 "4434d75a200373175b2613ca0c626b67ed982c72eade337a16d976e8a5a99825"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-x64.tar.gz"
      sha256 "27fd512fff67939579fe3baa0f260121deae08cfd389c582113888683615f475"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-arm64.tar.gz"
      sha256 "f58e87a927b07bd5b8794b69b8e15cb93ad2ee40c9c0a6a9b67b32309809d12d"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-x64.tar.gz"
      sha256 "70dcf55af6cd4ddfb0f6841c3ff685f19d094d37d3f4858c7b8d8283c1b37ab9"
    end
  end

  def install
    bin.install "turbotokens"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/turbotokens --version")
  end
end
