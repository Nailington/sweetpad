class SweetpadNailington < Formula
  desc "Nailington's SweetPad fork for local and remote Apple development"
  homepage "https://github.com/Nailington/sweetpad"
  version "0.1.12-nailington.4"
  license "MIT"

  on_macos do
    url "https://github.com/Nailington/sweetpad/releases/download/cli-v0.1.12-nailington.4/sweetpad-cli-0.1.12-nailington.4-macos-universal.tar.gz"
    sha256 "58428df00e536249c7a10231221ec17b780cd45f14ba3e5a4f57c81b718c1c39"
  end

  on_linux do
    depends_on arch: :x86_64
    url "https://github.com/Nailington/sweetpad/releases/download/cli-v0.1.12-nailington.4/sweetpad-cli-0.1.12-nailington.4-linux-x64.tar.gz"
    sha256 "edf26a9cbcffbcb2da871f8adfab8e796fe8dbdcfcf3ecd7d13f058ad4f180ae"
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
