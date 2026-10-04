/*
 * Source catalog (English). Every other locale mirrors these keys; the
 * parity test fails on a missing key or a mismatched {placeholder}.
 *
 * Conventions (docs/I18N.md):
 * - Keys name the place, not the words: `settings.rail.you`, not `you`.
 * - Whole sentences, never concatenated fragments: word order differs.
 * - Counts use plural forms ({ one, other }) and a {count} placeholder.
 * - Brand and technical tokens (Lain, HLS, Ctrl) stay inside the string.
 */
const en = {
	common: {
		appName: 'Lain',
		home: 'Lain home',
		saved: 'saved',
		cancel: 'Cancel',
		close: 'Close',
		signOut: 'Sign out'
	},
	nav: {
		primary: 'Primary',
		home: 'Home',
		library: 'Library',
		list: 'My list',
		social: 'Social',
		unread: { one: '{count} unread notification', other: '{count} unread notifications' },
		search: 'Search',
		settings: 'Settings',
		accountMenu: 'Account menu',
		palette: 'Open command palette (Ctrl+K)',
		scanning: 'scanning',
		skip: 'Skip to content'
	},
	boot: {
		connecting: 'Connecting to your server…',
		retry: 'Try again'
	},
	userMenu: {
		profile: 'Profile',
		playback: 'Playback',
		allSettings: 'All settings'
	},
	time: {
		justNow: 'just now'
	},
	settings: {
		title: 'Settings',
		sections: 'Settings sections',
		scope: {
			you: 'You',
			server: 'Server'
		},
		breadcrumb: 'settings / {scope} / {section}',
		hints: {
			find: 'find a setting',
			move: 'move',
			section: 'section'
		},
		saved: 'saved',
		staged: {
			region: 'Unsaved changes',
			count: { one: '{count} change staged', other: '{count} changes staged' },
			discard: 'Discard',
			apply: 'Apply'
		},
		section: {
			profile: { label: 'Profile', hint: 'Name, bio, avatar, account' },
			playback: { label: 'Playback', hint: 'Player, language, video effects' },
			connections: { label: 'Connections', hint: 'AniList and other lists' },
			security: { label: 'Security', hint: 'Password and sessions' },
			privacy: { label: 'Privacy', hint: 'Who sees your profile and activity' },
			libraries: { label: 'Libraries', hint: 'Media folders and scans' },
			users: { label: 'Users', hint: 'Accounts and roles' },
			transcoding: { label: 'Transcoding', hint: 'How files become streams' },
			integrations: { label: 'Integrations', hint: 'OAuth apps for list sync' },
			plugins: { label: 'Plugins', hint: 'Which provider does each job' },
			backup: { label: 'Backup', hint: 'Database snapshot' }
		},
		entry: {
			displayName: 'Display name',
			bio: 'Bio',
			avatar: 'Avatar',
			language: 'Interface language',
			signOut: 'Sign out',
			player: 'Default player',
			playbackLanguage: 'Audio & subtitle language',
			effects: 'Video effects',
			anilist: 'AniList',
			password: 'Change password',
			privacyProfile: 'Profile visibility',
			privacyActivity: 'Activity visibility',
			privacyRatings: 'Ratings visibility',
			discoverable: 'Appear in people search',
			friendRequests: 'Friend requests',
			favorites: 'Favorites',
			libraries: 'Media libraries',
			scan: 'Library scan',
			users: 'Users',
			mode: 'Transcoding mode',
			delivery: 'Delivery (HLS, segments)',
			encoding: 'Encoding & quality ladder',
			hardware: 'Hardware acceleration',
			processing: 'HDR & tone mapping',
			audio: 'Audio & subtitles (server)',
			resources: 'Resources & storage',
			sessions: 'Active transcode sessions',
			integrations: 'AniList OAuth app',
			plugins: 'Plugins & capabilities',
			backup: 'Download backup'
		},
		interface: {
			group: 'Interface',
			language: 'Language',
			languageHint: 'The language of Lain’s interface in this browser.',
			system: 'System ({name})'
		}
	},
	palette: {
		label: 'Command palette',
		placeholder: 'Search settings, actions and titles',
		results: 'Results',
		empty: 'Nothing matches “{query}”.',
		move: 'move',
		run: 'run',
		toggle: 'toggle',
		group: {
			actions: 'Actions',
			goTo: 'Go to',
			settings: 'Settings',
			titles: 'Titles'
		},
		action: {
			scan: 'Scan libraries',
			scanDetail: 'Walk every root and update the catalog',
			scanStarted: 'Scan started.',
			scanFailed: 'Could not start the scan.',
			addLibrary: 'Add library',
			addUser: 'Add user',
			backup: 'Download backup',
			signOut: 'Sign out'
		},
		sectionPrefix: 'Settings › {section}'
	},
	social: {
		pageTitle: 'Social — Lain',
		heading: 'Social',
		subtitle: 'Friends on this server: what they watch and read, what they rate, what they share with you.',
		privacyLink: 'Privacy settings',
		loading: 'Loading…',
		loadFailed: 'Could not load this view.',
		removedUser: 'Removed account',
		score: '{score}/10',
		tab: {
			feed: 'Feed',
			friends: 'Friends',
			notifications: 'Notifications',
			collections: 'Collections'
		},
		kind: {
			anime: 'Anime',
			series: 'Series',
			movie: 'Movie',
			manga: 'Manga',
			comic: 'Comic'
		},
		unit: {
			episode: 'S{season}E{episode}',
			chapter: 'Ch {n}',
			volume: 'Vol {n}'
		},
		visibility: {
			public: 'Everyone on this server',
			friends: 'Friends',
			private: 'Only me'
		},
		relation: {
			none: 'Not connected',
			friend: 'Friend',
			outgoing: 'Request sent',
			incoming: 'Wants to be friends',
			blocking: 'Blocked'
		},
		activity: {
			watching: 'Watching',
			reading: 'Reading',
			watched: 'Watched',
			read: 'Finished',
			rated: 'Rated',
			reviewed: 'Reviewed'
		},
		feed: {
			emptyTitle: 'Nothing from your friends yet',
			emptyBody: 'Add friends to see what they watch, read and rate. Their privacy settings decide what shows here.',
			findFriends: 'Find people',
			more: 'Load older'
		},
		friends: {
			searchLabel: 'Search people',
			searchPlaceholder: 'Search people by name — press / from anywhere on this page',
			searchFailed: 'Could not search people.',
			noResults: 'Nobody matches.',
			add: 'Add friend',
			accept: 'Accept',
			decline: 'Decline',
			cancel: 'Cancel request',
			remove: 'Remove friend',
			block: 'Block',
			unblock: 'Unblock',
			failed: 'That did not work.',
			incoming: 'Requests',
			outgoing: 'Sent requests',
			blocked: 'Blocked',
			list: { one: '{count} friend', other: '{count} friends' },
			none: 'No friends yet.',
			since: 'friends since {when}'
		},
		notify: {
			friendRequest: 'Friend request',
			friendAccepted: 'Accepted',
			share: 'Shared',
			collectionShared: 'Shared a collection',
			reply: 'Replied',
			openCollection: 'Open the collection',
			unread: { one: '{count} unread', other: '{count} unread' },
			markAll: 'Mark all read',
			empty: 'No notifications.',
			failed: 'Could not update notifications.'
		},
		title: {
			heading: 'Community',
			scored: { one: 'from {count} rating you can see', other: 'from {count} ratings you can see' },
			unrated: 'No ratings you can see yet.',
			rate: 'Rate',
			yourScore: 'Your score: {score}',
			editReview: 'Edit review',
			share: 'Share',
			collect: 'Add to collection',
			favorite: 'Favorite',
			addFavorite: 'Add to favorites',
			favorited: 'Added to your favorites.',
			unfavorited: 'Removed from your favorites.',
			favoriteFailed: 'Could not change your favorites.',
			reviews: 'Reviews',
			revealSpoiler: 'Contains spoilers — show',
			loadFailed: 'Could not load ratings and comments.'
		},
		rate: {
			title: 'Rate {title}',
			description: 'Your friends see it unless your privacy settings say otherwise.',
			score: 'Score',
			keys: '1–9 pick a score, 0 is 10, Ctrl+Enter saves',
			review: 'Review (optional)',
			reviewPlaceholder: 'What stayed with you?',
			spoiler: 'Contains spoilers',
			save: 'Save',
			remove: 'Remove',
			failed: 'Could not save your rating.'
		},
		comments: {
			heading: { one: '{count} comment', other: '{count} comments' },
			empty: 'No comments yet.',
			label: 'Comment',
			placeholder: 'Say something about this title — everyone on this server can read it',
			post: 'Post',
			reply: 'Reply',
			delete: 'Delete',
			replyingTo: 'Replying to {name}',
			focusHint: 'to write',
			failed: 'Could not post the comment.',
			deleteFailed: 'Could not delete the comment.'
		},
		share: {
			workTitle: 'Share {title}',
			collectionTitle: 'Share {name}',
			description: 'Friends get a notification. Only friends can receive shares.',
			recipients: 'Friends',
			picked: 'Selected',
			message: 'Message (optional)',
			messagePlaceholder: 'Why they should look',
			noFriends: 'You have no friends to share with yet.',
			send: { one: 'Send to {count} friend', other: 'Send to {count} friends' },
			sent: { one: 'Shared with {count} friend.', other: 'Shared with {count} friends.' },
			failed: 'Could not share.',
			loadFailed: 'Could not load your friends.'
		},
		collect: {
			title: 'Add to collection',
			description: 'Toggle {title} in your collections, or start a new one.',
			count: { one: '{count} title', other: '{count} titles' },
			in: 'Added',
			none: 'You have no collections yet.',
			newLabel: 'New collection name',
			newPlaceholder: 'New collection',
			create: 'Create',
			created: 'Created {name}.',
			failed: 'Could not change the collection.',
			loadFailed: 'Could not load your collections.'
		},
		collections: {
			pageTitle: '{name} — Lain',
			kicker: 'Collection',
			new: 'New collection',
			newDescription: 'A named list of titles of any kind, private until you share it.',
			name: 'Name',
			descriptionField: 'Description',
			visibility: 'Who can see it',
			create: 'Create',
			createFailed: 'Could not create the collection.',
			empty: 'No collections yet — yours and the ones friends share with you appear here.',
			sharedBy: 'shared by {name}',
			edit: 'Edit',
			save: 'Save',
			delete: 'Delete',
			deleteTitle: 'Delete {name}?',
			deleteBody: 'The collection is removed for everyone it was shared with. The titles themselves are untouched.',
			removeItem: 'Remove',
			noItems: 'Nothing here yet. Add titles from their page with C.',
			sharedWith: 'Shared with',
			unshare: 'Stop sharing',
			loadFailed: 'Could not load the collection.',
			saveFailed: 'Could not save the collection.',
			deleteFailed: 'Could not delete the collection.',
			missingTitle: 'Collection not found',
			missingBody: 'It was deleted, or it is not shared with you.'
		},
		profile: {
			pageTitle: '{name} — Lain',
			edit: 'Edit profile',
			favorites: 'Favorites',
			activity: 'Activity',
			ratings: 'Ratings',
			collections: 'Collections',
			private: 'Not shared with you.',
			noFavorites: 'No favorites yet.',
			noActivity: 'No activity yet.',
			noRatings: 'No ratings yet.',
			noCollections: 'No collections to show.',
			spoilerHidden: 'Review hidden: contains spoilers.',
			loadFailed: 'Could not load this profile.',
			missingTitle: 'No such member',
			missingBody: 'This account does not exist or is not available to you.'
		},
		block: {
			title: 'Block {name}?',
			description: 'You stop seeing each other’s profile, activity, ratings and comments, and they cannot send you requests or shares. They are not told.'
		},
		privacy: {
			pageTitle: 'Privacy — Lain',
			visibilityGroup: 'Visibility',
			profile: 'Profile',
			profileHint: 'Bio, favorites and your collections list.',
			activity: 'Activity',
			activityHint: 'What you watch, read and finish, in friends’ feeds and on your profile.',
			hideProgress: 'Hide what I am in the middle of',
			hideProgressHint: 'Show only finished titles and ratings, never “watching” or “reading”.',
			ratings: 'Ratings and reviews',
			ratingsHint: 'Your scores and reviews on title pages and your profile.',
			peopleGroup: 'People',
			discoverable: 'Appear in people search',
			discoverableHint: 'Friends and pending requests still find you when this is off.',
			requests: 'Friend requests',
			requestsHint: 'Who may send you a request.',
			requestsEveryone: 'Everyone',
			requestsNobody: 'Nobody',
			blocked: 'Blocked accounts',
			blockedHint: 'Blocks are listed under Social › Friends.',
			manage: 'Manage',
			favoritesGroup: 'Profile',
			favorites: 'Favorites',
			favoritesHint: 'Up to 12, shown on your profile. Press F on any title page to add one.',
			removeFavorite: 'Remove',
			noFavorites: 'No favorites yet.',
			viewProfile: 'Your public profile',
			viewProfileHint: 'See it as others do.',
			open: 'Open',
			loadFailed: 'Could not load privacy settings.',
			saveFailed: 'Could not save the setting.'
		}
	}
} as const;

export default en;
