# SPDX-License-Identifier: AGPL-3.0-or-later
# Optional developer overrides for every image this flake builds. omaflake already passes your Mac's values
# (user, full name, time zone, locale, keyboard, SSH keys, macOS user of the base – flake input omaflake-guest);
# everything set here overrides them. Copy the whole folder: cp -R local.example local   (local/ is git-ignored)
{
  user = "me";                  # login user inside the guests (also used by `omaf ssh`)
  fullName = "Your Name";       # account description and git user.name in the guests
  email = null;                 # git user.email in the guests, e.g. "you@example.com"
  timeZone = "UTC";             # e.g. "Europe/Berlin"
  locale = "en_US.UTF-8";       # e.g. "de_DE.UTF-8"
  keyboard = "us";              # console and Hyprland keyboard layout, e.g. "de"
  macUser = null;               # macOS guest user if it differs from `user` (it is fixed by the base)
  # macKeyboardLayout = { id = -18133; name = "German-DIN-2137"; };  # macOS layout if `keyboard` is not "de"/"us"
}
