class Cranibox < Formula
  desc "Give your agents hands on every device you own"
  homepage "https://github.com/fliaping/cranibox"
  version "1.2.1"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/fliaping/cranibox/releases/download/v1.2.1/cranibox-macos-arm64"
      sha256 "afdb1d30e1f650de8eef3ecaec69214310aa5ca51c4ffdfa6becf4d338bf53ef"
    end
  end

  def install
    bin.install "cranibox-macos-arm64" => "cranibox"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cranibox --version")
  end
end
