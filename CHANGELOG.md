# Changelog

All notable changes to Soplos Plymouth Manager are documented in this file.

## [3.0.0-1-beta] - 2026-10-28

### Changed

- New application icon, in the sizes of the package, the window and the interface.
- New hero image, with the circuit style of the other Soplos 3.0 programs.
- New screenshots of the gallery and of the preview, taken with the interface in English.

## [3.0.0-beta] - 2026-10-25

First beta, after the alpha was tested on real hardware.

## [3.0.0-alpha] - 2026-10-24

Full rewrite of Soplos Plymouth Manager, from Python 3 and GTK3 to Tauri 2 with a Rust backend and a Svelte frontend. See the Architecture section of the README. Not yet tested on real hardware.

### Added

- Rust backend (`src-tauri/`) replacing the Python 3 and GTK3 codebase. It reads the Plymouth themes installed in `/usr/share/plymouth/themes`, `/lib/plymouth/themes` and `/usr/local/share/plymouth/themes`, the theme in use (from `/etc/plymouth/plymouthd.conf`, or from the default link when the file does not name one), the desktop, the display protocol and the initramfs tool. A theme is graphical unless its module is `text` or `details`; a theme that names no module is graphical when its folder holds images or a script.
- Svelte 4 and Tailwind CSS frontend (`src/`): one window, as in the 2.x program, with the frame of the other Soplos 3.0 programs that have no sidebar, a hero with the language, a thin footer with the theme button, the session and the version.
- The gallery of themes of the 2.x program: a card for each theme with its image (read when the card is shown, so the list does not carry every image at once) and its name, the theme in use marked, and the buttons under it to apply, preview, install, remove and refresh. The theme in use cannot be removed and a text-only theme cannot be previewed.
- Several themes can be selected at once, with Ctrl or Shift click or the box in the corner of a card, and removed together after one question; the theme in use is left out.
- Support for dracut (what Soplos uses) and for initramfs-tools (Debian), and detection of mkinitcpio. When more than one is installed, dracut is used.
- Real actions, run as jobs with a progress bar along the bottom edge of the window; a failure is shown in a dialog with what the tools said. Apply sets the default theme (`plymouth-set-default-theme`) and rebuilds the initramfs (`dracut --regenerate-all --force`, `update-initramfs -u` or `mkinitcpio -P`). Install unpacks a `.zip`, `.tar.gz`, `.tar.xz` or `.tar.bz2` archive, checks that nothing in it escapes the folder and copies the theme to `/usr/share/plymouth/themes`. Remove deletes the selected themes; the theme in use and the base themes (text, details, spinner, bgrt) are never removed.
- Privileged helper: the same executable started through `pkexec` with a closed list of operations in a private file, validated in the window and again as the superuser. One password per action.
- Real preview: Plymouth is shown in a nested Xephyr display (or, for the themes that draw on the text console, in a full-screen terminal), a picture is taken once the splash is up and installed by the helper as the `preview.png` of the theme folder, as in the 2.x program, so it goes away with the theme. It works from X11 and from Wayland through XWayland, and the theme that was in use is put back afterwards.
- Keyboard shortcuts of the 2.x program: F1, Ctrl+Q, F5 and Ctrl+R, Ctrl+A, Ctrl+P, Ctrl+I and Delete.
- The window follows the light or dark theme of the desktop, with a button in the footer to choose.
- Interface in eight languages: English, Spanish, German, French, Italian, Portuguese, Romanian and Russian. It starts in the system language and falls back to English.
- Unix manual page (`soplos-plymouth-manager.1`) installed to `/usr/share/man/man1/`, AppStream metainfo and desktop entry in the eight languages, Polkit policy for the helper of the next phase, Debian copyright (declaring Lucide, ISC) and this `CHANGELOG.md` installed as `/usr/share/doc/soplos-plymouth-manager/changelog.gz`.
