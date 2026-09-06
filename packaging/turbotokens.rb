class Turbotokens < Formula
  desc "Real-time token and cost telemetry for AI coding agents"
  homepage "https://github.com/maxmoneycash/turbotokens"
  version "1.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-arm64.tar.gz"
      sha256 "a6f10924276bf1e84cd7d79c3355b5a8ed10bbe6c41e6cf7f7c8978a473e6ea0"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-macos-x64.tar.gz"
      sha256 "26e6778c92655fc3b55c245237b249d888078d13b656f8ac52f91717508db44a"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-arm64.tar.gz"
      sha256 "be65eb4de2ef8c3d75d4ce37a6cfa7d657e23aa57b28542efe6ddc9025d288c8"
    end
    on_intel do
      url "https://github.com/maxmoneycash/turbotokens/releases/download/v#{version}/turbotokens-linux-x64.tar.gz"
      sha256 "26f4c926d3c35515c963c011c41e157f326ccdf81bace68c993f71f181f1bb3a"
    end
  end

  def install
    bin.install "turbotokens"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/turbotokens --version")
  end
end
