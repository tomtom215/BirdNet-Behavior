# Homebrew formula for BirdNet-Behavior (Apple Silicon).
#
# STATUS: template — pending a hardware-verified `aarch64-apple-darwin` release.
# The release pipeline (.github/workflows/release.yml) already builds and
# uploads `birdnet-behavior-<version>-aarch64-apple-darwin.tar.gz`, and lists its
# SHA-256 in the release's `SHA256SUMS` (the per-archive `.sha256` sidecars are
# build artefacts only; they are deleted before the release is assembled). Once
# a release is cut AND verified on real M-series hardware, fill in `version` and
# `sha256` (the line for that archive in `SHA256SUMS`) and publish this file in
# a tap repo, e.g.:
#
#     tomtom215/homebrew-birdnet-behavior  ->  Formula/birdnet-behavior.rb
#     brew install tomtom215/birdnet-behavior/birdnet-behavior
#
# until then, build from source (see docs/book/field/macos.md).
class BirdnetBehavior < Formula
  desc "Real-time acoustic bird classification with DuckDB behavioral analytics"
  homepage "https://github.com/tomtom215/BirdNet-Behavior"
  version "0.2.0"
  license "CC-BY-NC-SA-4.0"

  # Apple Silicon only: ort ships an aarch64-apple-darwin ONNX Runtime prebuilt.
  on_macos do
    on_arm do
      url "https://github.com/tomtom215/BirdNet-Behavior/releases/download/v#{version}/birdnet-behavior-#{version}-aarch64-apple-darwin.tar.gz"
      # Replace with the archive's line in the release's SHA256SUMS:
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
    # The licence terms travel with the binary, as they do in every release
    # archive: NOTICE and THIRD_PARTY_LICENSES.txt carry the third-party terms
    # the statically linked code requires to be reproduced.
    doc.install "README.md", "CHANGELOG.md", "LICENSE", "LICENSE-UPSTREAM", "NOTICE", "THIRD_PARTY_LICENSES.txt"
  end

  def caveats
    <<~EOS
      BirdNet-Behavior captures the microphone via ffmpeg's avfoundation input,
      which needs Microphone access granted to the controlling process under
      System Settings -> Privacy & Security -> Microphone. A headless service
      cannot obtain that consent, so run it as a per-user LaunchAgent (see the
      plist in #{doc}/.. or the repo's packaging/macos/) rather than `brew services`.

      The binary does not download a model on its own. install.sh does it for you
      (BirdNET V2.4 by default, ~85 MB, the network BirdNET-Pi and birdnet-go
      run); by hand, with a directory you own:
        BIRDNET_MODEL_DIR=~/models birdnet-behavior --install-model birdnet-v24
        BIRDNET_MODEL_DIR=~/models birdnet-behavior --install-model birdnet-v24-range-v2
      then set MODEL_PATH, LABELS_PATH and METADATA_MODEL_PATH in birdnet.conf to
      the files it prints.

      Configure your station before first run: create
        #{etc}/birdnet-behavior/birdnet.conf
      with at least LATITUDE= and LONGITUDE= (the manual's Configuration page lists
      every key; no example file is installed), then start the LaunchAgent.
    EOS
  end

  test do
    assert_match "birdnet-behavior", shell_output("#{bin}/birdnet-behavior --help")
  end
end
