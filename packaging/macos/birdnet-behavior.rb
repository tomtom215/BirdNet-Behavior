# Homebrew formula for BirdNet-Behavior (Apple Silicon).
#
# STATUS: template — pending a hardware-verified `aarch64-apple-darwin` release.
# The release pipeline (.github/workflows/release.yml) already builds and
# uploads `birdnet-behavior-<version>-aarch64-apple-darwin.tar.gz` plus a
# `.sha256` sidecar. Once a release is cut AND verified on real M-series
# hardware, fill in `version` and `sha256` (copy the value from the sidecar)
# and publish this file in a tap repo, e.g.:
#
#     tomtom215/homebrew-birdnet-behavior  ->  Formula/birdnet-behavior.rb
#     brew install tomtom215/birdnet-behavior/birdnet-behavior
#
# until then, build from source (see docs/MACOS.md).
class BirdnetBehavior < Formula
  desc "Real-time acoustic bird classification with DuckDB behavioral analytics"
  homepage "https://github.com/tomtom215/BirdNet-Behavior"
  version "0.2.0"
  license "CC-BY-NC-SA-4.0"

  # Apple Silicon only: ort ships an aarch64-apple-darwin ONNX Runtime prebuilt.
  on_macos do
    on_arm do
      url "https://github.com/tomtom215/BirdNet-Behavior/releases/download/v#{version}/birdnet-behavior-#{version}-aarch64-apple-darwin.tar.gz"
      # Replace with the value from the release's .tar.gz.sha256 sidecar:
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  # ffmpeg captures the microphone (avfoundation) and RTSP streams.
  depends_on "ffmpeg"
  depends_on arch: :arm64
  depends_on :macos

  def install
    bin.install "birdnet-behavior"
    (etc/"birdnet-behavior").mkpath
    (var/"birdnet-behavior").mkpath
    (var/"log").mkpath
    doc.install "README.md", "CHANGELOG.md", "LICENSE", "LICENSE-UPSTREAM"
  end

  def caveats
    <<~EOS
      BirdNet-Behavior captures the microphone via ffmpeg's avfoundation input,
      which needs Microphone access granted to the controlling process under
      System Settings -> Privacy & Security -> Microphone. A headless service
      cannot obtain that consent, so run it as a per-user LaunchAgent (see the
      plist in #{doc}/.. or the repo's packaging/macos/) rather than `brew services`.

      First run downloads the BirdNET+ V3.0 model (~541 MB) from Zenodo.

      Configure your station before first run:
        cp #{etc}/birdnet-behavior/birdnet.conf.example #{etc}/birdnet-behavior/birdnet.conf
        # edit LATITUDE/LONGITUDE, then start the LaunchAgent.
    EOS
  end

  test do
    assert_match "birdnet-behavior", shell_output("#{bin}/birdnet-behavior --help")
  end
end
