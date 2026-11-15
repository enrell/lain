import { api, ApiError } from '$lib/api';
import type { User } from '$lib/api/types';
import { configureApi } from '$lib/api/client';
import { canManageRole, isAdminRole, isManagerRole } from './roles';
import { rememberAccount, tokenStorage } from './storage';

/*
 * The single global state that genuinely is global: who is signed in.
 * Everything else (pages, players, forms) keeps local state.
 *
 * Boot sequence:
 *   1. GET /api/setup/status  → first-run setup or login
 *   2. stored token? GET /api/me validates it before we trust it
 *   3. 401 anywhere clears local state; the layout redirects to /login
 */
class Session {
	token = $state<string | null>(null);
	user = $state<User | null>(null);
	/** True once the boot sequence settled (success or definitive failure). */
	ready = $state(false);
	/** True while a first-run setup is required by the server. */
	setupRequired = $state(false);
	/** True between completing setup and leaving the welcome step. */
	freshSetup = $state(false);
	/** Non-null when the server could not be reached during boot. */
	bootError = $state<string | null>(null);
	busy = $state(false);

	constructor() {
		configureApi({
			getToken: () => this.token,
			onUnauthorized: () => this.invalidate()
		});
	}

	get authenticated(): boolean {
		return this.user !== null;
	}

	get isAdmin(): boolean {
		return isAdminRole(this.user?.role);
	}

	/** The manager role: library care, not the keys to the server (D-296). */
	get isManager(): boolean {
		return isManagerRole(this.user?.role);
	}

	/** Admin or manager. An unknown role is neither. */
	get canManage(): boolean {
		return canManageRole(this.user?.role);
	}

	async bootstrap(): Promise<void> {
		this.bootError = null;
		try {
			const status = await api.auth.setupStatus();
			this.setupRequired = status.setup_required;
			if (this.setupRequired) {
				this.invalidate();
				this.ready = true;
				return;
			}
			const saved = tokenStorage.load();
			if (saved) {
				this.token = saved;
				try {
					this.user = await api.me.get();
					rememberSignedIn(this.user);
				} catch {
					this.invalidate();
				}
			}
			this.ready = true;
		} catch (err) {
			this.bootError =
				err instanceof ApiError && err.kind === 'network'
					? 'Cannot reach the Lain server.'
					: 'The server answered with an unexpected error.';
		}
	}

	async retry(): Promise<void> {
		this.ready = false;
		if (this.token === null) tokenStorage.clear();
		await this.bootstrap();
	}

	async login(username: string, password: string): Promise<void> {
		this.busy = true;
		try {
			const { token } = await api.auth.login(username, password);
			this.token = token;
			tokenStorage.save(token);
			this.user = await api.me.get();
			rememberSignedIn(this.user);
			this.setupRequired = false;
			this.bootError = null;
		} catch (err) {
			this.invalidate();
			throw err;
		} finally {
			this.busy = false;
		}
	}

	/** First-run: create the admin, then establish a session with it. */
	async setup(username: string, password: string): Promise<void> {
		this.busy = true;
		try {
			await api.auth.setup(username, password);
		} finally {
			this.busy = false;
		}
		// Set before login: the guard must not bounce the welcome step
		// while `user` flips to authenticated mid-login.
		this.freshSetup = true;
		try {
			await this.login(username, password);
		} catch (err) {
			this.freshSetup = false;
			throw err;
		}
		this.setupRequired = false;
	}

	/** Leaves the post-setup welcome screen (the guard then applies). */
	acknowledgeSetup(): void {
		this.freshSetup = false;
	}

	async refreshUser(): Promise<void> {
		this.user = await api.me.get();
		rememberSignedIn(this.user);
	}

	/**
	 * Clears client auth state and sends nothing (a 401 lands here: the
	 * server already refuses the token). The account tile is written
	 * first.
	 */
	invalidate(): void {
		rememberSignedIn(this.user);
		this.token = null;
		this.user = null;
		this.freshSetup = false;
		tokenStorage.clear();
	}

	/**
	 * Signs out (D-298): the page forgets the session at once, and the
	 * server is told to refuse its token from now on. Unreachable, the
	 * server keeps the token until it expires; nothing here waits on it.
	 */
	logout(): void {
		const token = this.token;
		this.invalidate();
		if (token) void api.auth.logout(token).catch(() => {});
	}
}

/** The tile for this browser. The token stays in its own key. */
function rememberSignedIn(user: User | null): void {
	if (!user) return;
	rememberAccount({
		id: user.id,
		username: user.username,
		displayName: user.profile?.display_name,
		avatar: user.profile?.avatar
	});
}

export const session = new Session();
