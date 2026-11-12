/*
 * The shell's keyboard-shortcuts dialog. `?` opens it, and so does the
 * account menu, for whoever has not met the key yet.
 */
class ShortcutsDialog {
	open = $state(false);
}

export const shortcutsDialog = new ShortcutsDialog();
