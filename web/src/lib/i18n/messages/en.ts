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
	}
} as const;

export default en;
