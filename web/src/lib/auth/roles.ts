import { t } from '$lib/i18n';

/** The three roles the server accepts. Anything else is an ordinary account. */
export const ROLE_ADMIN = 'admin';
export const ROLE_MANAGER = 'manager';
export const ROLE_USER = 'user';

export function isAdminRole(role: string | null | undefined): boolean {
	return role === ROLE_ADMIN;
}

export function isManagerRole(role: string | null | undefined): boolean {
	return role === ROLE_MANAGER;
}

/** Renaming and ordering libraries, and scans (D-296). */
export function canManageRole(role: string | null | undefined): boolean {
	return role === ROLE_ADMIN || role === ROLE_MANAGER;
}

/**
 * What to print for a role. An unknown role reads as an ordinary user:
 * it never inherits the admin label.
 */
export function roleLabel(role: string | null | undefined): string {
	if (role === ROLE_ADMIN) return t('settings.usersPage.roleAdmin');
	if (role === ROLE_MANAGER) return t('settings.usersPage.roleManager');
	return t('settings.usersPage.roleUser');
}
