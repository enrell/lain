import { describe, expect, it } from 'vitest';
import { canManageRole, isAdminRole, isManagerRole, roleLabel } from './roles';

describe('roles', () => {
	it('keeps admin, manager and an unknown role apart', () => {
		expect(isAdminRole('admin')).toBe(true);
		expect(isAdminRole('manager')).toBe(false);
		expect(isAdminRole('user')).toBe(false);
		expect(isAdminRole('root')).toBe(false);
		expect(isAdminRole(undefined)).toBe(false);

		expect(isManagerRole('manager')).toBe(true);
		expect(isManagerRole('admin')).toBe(false);
		expect(isManagerRole('nope')).toBe(false);

		expect(canManageRole('admin')).toBe(true);
		expect(canManageRole('manager')).toBe(true);
		expect(canManageRole('user')).toBe(false);
		expect(canManageRole('')).toBe(false);
		expect(canManageRole(null)).toBe(false);
	});

	it('labels an unknown role as an ordinary user', () => {
		expect(roleLabel('admin')).toBe('Admin');
		expect(roleLabel('manager')).toBe('Manager');
		expect(roleLabel('user')).toBe('User');
		expect(roleLabel('root')).toBe('User');
		expect(roleLabel(undefined)).toBe('User');
	});
});
