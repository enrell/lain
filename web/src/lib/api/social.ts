import { request } from './client';
import type { UserProfile } from './types';

/*
 * Social slice (docs/slices/social.md). Shapes mirror
 * internal/contracts/social.go and internal/gateway/social.go. Responses
 * that mention other accounts carry a `users` map of public summaries;
 * an id missing from it is a removed or disabled account.
 */

export type Visibility = 'public' | 'friends' | 'private';
export type Relation = 'none' | 'friend' | 'outgoing' | 'incoming' | 'blocking';

/** A work of any media kind: anime, series, movie, comic, manga, … */
export interface WorkRef {
	kind: string;
	title: string;
	item_id?: string;
}

/** Name a work by catalog item, or by kind + title when there is no file. */
export type WorkTarget = { item_id: string } | { kind: string; title: string };

export interface UserSummary {
	id: string;
	username: string;
	display_name?: string;
	avatar: UserProfile['avatar'];
}

export type UserMap = Record<string, UserSummary>;

export interface SocialSettings {
	user_id: string;
	profile: Visibility;
	activity: Visibility;
	ratings: Visibility;
	discoverable: boolean;
	allow_requests: 'everyone' | 'nobody';
	hide_progress: boolean;
	favorites: WorkRef[];
	updated_at?: number;
}

export type SettingsPatch = Partial<Omit<SocialSettings, 'user_id' | 'updated_at' | 'favorites'>> & {
	favorites?: WorkTarget[];
};

export interface Activity {
	id: string;
	user_id: string;
	type: 'progress' | 'completed' | 'rated';
	work: WorkRef;
	season?: number;
	episode?: number;
	score?: number;
	at: number;
}

export interface Notification {
	id: string;
	user_id: string;
	type: 'friend_request' | 'friend_accepted' | 'share' | 'collection_shared' | 'reply';
	from_user_id: string;
	work?: WorkRef;
	collection_id?: string;
	comment_id?: string;
	message?: string;
	read: boolean;
	at: number;
}

export interface Rating {
	user_id: string;
	work: WorkRef;
	score?: number;
	review?: string;
	spoiler?: boolean;
	created_at: number;
	updated_at: number;
}

export interface Comment {
	id: string;
	user_id: string;
	work: WorkRef;
	body: string;
	reply_to?: string;
	created_at: number;
}

export interface CollectionItem {
	work: WorkRef;
	note?: string;
	added_at: number;
}

export interface Collection {
	id: string;
	owner_id: string;
	name: string;
	description?: string;
	visibility: Visibility;
	shared_with: string[];
	items: CollectionItem[];
	created_at: number;
	updated_at: number;
}

export interface WorkView {
	work: WorkRef;
	mine?: Rating | null;
	ratings: Rating[];
	average: number;
	scored: number;
	comments: Comment[];
	users: UserMap;
}

export interface ProfileView {
	user: UserSummary;
	relation: Relation;
	self: boolean;
	visible: { profile: boolean; activity: boolean; ratings: boolean };
	bio?: string;
	favorites: WorkRef[];
}

export interface RelationRow {
	user: UserSummary;
	since: number;
}

export interface FriendsView {
	friends: RelationRow[];
	incoming: RelationRow[];
	outgoing: RelationRow[];
	blocked: RelationRow[];
}

const seg = encodeURIComponent;

function workQuery(w: WorkTarget): Record<string, string> {
	return 'item_id' in w ? { item: w.item_id } : { kind: w.kind, title: w.title };
}

/** A WorkRef as a target: by item when it has one, else kind + title. */
export function targetOf(w: WorkRef): WorkTarget {
	return w.item_id ? { item_id: w.item_id } : { kind: w.kind, title: w.title };
}

export const social = {
	settings: () => request<SocialSettings>('/api/social/me/settings'),
	updateSettings: (patch: SettingsPatch) =>
		request<SocialSettings>('/api/social/me/settings', { method: 'PATCH', body: patch }),

	searchUsers: (q: string) =>
		request<{ users: { user: UserSummary; relation: Relation }[] }>('/api/social/users', { query: { q } }),
	profile: (id: string) => request<ProfileView>(`/api/social/users/${seg(id)}`),
	userActivity: (id: string, before?: number) =>
		request<{ items: Activity[]; users: UserMap }>(`/api/social/users/${seg(id)}/activity`, {
			query: { before }
		}),
	userRatings: (id: string) => request<{ ratings: Rating[] }>(`/api/social/users/${seg(id)}/ratings`),
	userCollections: (id: string) =>
		request<{ collections: Collection[]; users: UserMap }>(`/api/social/users/${seg(id)}/collections`),

	friends: () => request<FriendsView>('/api/social/friends'),
	/** Send a request, or accept one that is waiting. */
	befriend: (id: string) =>
		request<{ relation: Relation }>(`/api/social/friends/${seg(id)}`, { method: 'POST' }),
	decline: (id: string) =>
		request<{ relation: Relation }>(`/api/social/friends/${seg(id)}/decline`, { method: 'POST' }),
	/** Unfriend, or cancel a request you sent. */
	unfriend: (id: string) =>
		request<{ relation: Relation }>(`/api/social/friends/${seg(id)}`, { method: 'DELETE' }),
	block: (id: string) => request<{ relation: Relation }>(`/api/social/blocks/${seg(id)}`, { method: 'POST' }),
	unblock: (id: string) =>
		request<{ relation: Relation }>(`/api/social/blocks/${seg(id)}`, { method: 'DELETE' }),

	feed: (before?: number) =>
		request<{ items: Activity[]; users: UserMap }>('/api/social/feed', { query: { before } }),
	notifications: (opts: { suppressAuthRedirect?: boolean } = {}) =>
		request<{ items: Notification[]; unread: number; users: UserMap }>('/api/social/notifications', opts),
	markRead: (body: { ids?: string[]; all?: boolean }) =>
		request<{ changed: number }>('/api/social/notifications/read', { method: 'POST', body }),

	work: (w: WorkTarget) => request<WorkView>('/api/social/work', { query: workQuery(w) }),
	rate: (w: WorkTarget, body: { score: number; review: string; spoiler: boolean }) =>
		request<Rating>('/api/social/work/rating', { method: 'PUT', body: { work: w, ...body } }),
	unrate: (w: WorkTarget) =>
		request<{ removed: boolean }>('/api/social/work/rating', { method: 'DELETE', query: workQuery(w) }),
	comment: (w: WorkTarget, body: string, replyTo?: string) =>
		request<Comment>('/api/social/work/comments', {
			method: 'POST',
			body: { work: w, body, reply_to: replyTo }
		}),
	deleteComment: (id: string) =>
		request<{ removed: boolean }>(`/api/social/comments/${seg(id)}`, { method: 'DELETE' }),
	share: (w: WorkTarget, to: string[], message: string) =>
		request<{ sent: string[] }>('/api/social/share', { method: 'POST', body: { work: w, to, message } }),

	collections: () => request<{ collections: Collection[]; users: UserMap }>('/api/social/collections'),
	collection: (id: string) =>
		request<{ collection: Collection; users: UserMap }>(`/api/social/collections/${seg(id)}`),
	createCollection: (body: { name: string; description?: string; visibility?: Visibility }) =>
		request<{ collection: Collection; users: UserMap }>('/api/social/collections', { method: 'POST', body }),
	updateCollection: (id: string, body: { name?: string; description?: string; visibility?: Visibility }) =>
		request<{ collection: Collection; users: UserMap }>(`/api/social/collections/${seg(id)}`, {
			method: 'PATCH',
			body
		}),
	deleteCollection: (id: string) =>
		request<{ removed: boolean }>(`/api/social/collections/${seg(id)}`, { method: 'DELETE' }),
	addToCollection: (id: string, w: WorkTarget, note = '') =>
		request<{ collection: Collection; users: UserMap }>(`/api/social/collections/${seg(id)}/items`, {
			method: 'POST',
			body: { work: w, note }
		}),
	removeFromCollection: (id: string, w: WorkTarget) =>
		request<{ collection: Collection; users: UserMap }>(`/api/social/collections/${seg(id)}/items`, {
			method: 'DELETE',
			query: workQuery(w)
		}),
	shareCollection: (id: string, to: string[], message: string) =>
		request<{ sent: string[] }>(`/api/social/collections/${seg(id)}/share`, {
			method: 'POST',
			body: { to, message }
		}),
	unshareCollection: (id: string, userId: string) =>
		request<{ collection: Collection; users: UserMap }>(
			`/api/social/collections/${seg(id)}/share/${seg(userId)}`,
			{ method: 'DELETE' }
		)
};
