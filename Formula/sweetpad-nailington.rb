class SweetpadNailington < Formula
  desc "Nailington's SweetPad fork for local and remote Apple development"
  homepage "https://github.com/Nailington/sweetpad"
  version "0.1.12-nailington.3"
  license "MIT"

  on_macos do
    url "https://github.com/Nailington/sweetpad/releases/download/cli-v0.1.12-nailington.3/sweetpad-cli-0.1.12-nailington.3-macos-universal.tar.gz"
    sha256 "1ee8c4b068acf38400ebaddf526fa0bbd82285e74bbae6183c09c460d1a7b274"
  end

  on_linux do
    depends_on arch: :x86_64
    url "https://github.com/Nailington/sweetpad/releases/download/cli-v0.1.12-nailington.3/sweetpad-cli-0.1.12-nailington.3-linux-x64.tar.gz"
    sha256 "d66260dabc45b279e899c3499c7b100e6bf8f452aa0602f30d0ef31d7e0d6273"
  end

  conflicts_with "sweetpad", because: "both install the sweetpad command"

  def install
    bin.install "sweetpad"
    bin.install_symlink bin/"sweetpad" => "sweetpad-nailington"
  end

  test do
    assert_match "sweetpad #{version}", shell_output("#{bin}/sweetpad --version")
  end
end
