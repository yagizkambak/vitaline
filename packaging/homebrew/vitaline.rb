# Homebrew cask template.
#
# This file doesn't live in THIS repo, it gets copied to a separate tap repo:
#   github.com/yagizkambak/homebrew-tap  ->  Casks/vitaline.rb
# The copy here is the source/reference; `version` and `sha256` get updated
# on every release and pushed to the tap (see packaging/README.md).
#
# WHY NOT THE OFFICIAL TAP: Homebrew removes unsigned/unnotarized casks from
# the official tap (deadline September 2026). Unless the app is signed with
# an Apple Developer ID, our own tap is the only viable path.
cask "vitaline" do
  version "0.2.0"
  sha256 "933101aeb0831423d63532cf1cd4830ed981ad190cdd7c9eaf9a6e3cd5125251"

  url "https://github.com/yagizkambak/vitaline/releases/download/v#{version}/Vitaline_#{version}_universal.dmg"
  name "Vitaline"
  desc "Shows GitLab/GitHub/Azure pipeline status in the macOS notch"
  homepage "https://github.com/yagizkambak/vitaline"

  # The app is unsigned. If the quarantine flag stays, Gatekeeper says "damaged,
  # move to Trash"; the user needs to run `xattr -cr` on it after install
  # (see caveats below). Homebrew removed --no-quarantine in 4.7, so there's
  # no way to skip this from the install command anymore.
  app "Vitaline.app"

  zap trash: [
    "~/Library/Application Support/dev.vitaline.desktop",
    "~/Library/Saved Application State/dev.vitaline.desktop.savedState",
  ]

  caveats <<~EOS
    The app isn't signed with an Apple Developer ID. If it won't open because
    of the quarantine flag:

      xattr -cr "/Applications/Vitaline.app"
  EOS
end
