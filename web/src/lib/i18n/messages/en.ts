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
			backup: { label: 'Backup', hint: 'Database snapshot' },
			downloads: { label: 'Downloads', hint: 'Fetch to server disk, limits' },
			offline: { label: 'Offline', hint: 'Saved in this browser' }
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
			backup: 'Download backup',
			downloadQueue: 'Download queue',
			downloadStorage: 'Download storage & cleanup',
			downloadLimits: 'Download limits & quota',
			offlineStorage: 'Offline storage in this browser'
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
	downloads: {
		title: 'Downloads — Settings — Lain',
		queue: {
			group: 'Queue',
			add: 'Add a download',
			addHint: 'An http(s) URL the server fetches onto its own disk.',
			url: 'URL',
			urlPlaceholder: 'https://example.test/files/Frieren - 01.mkv',
			name: 'File name (optional)',
			namePlaceholder: 'From the URL or the server',
			target: 'Destination',
			targetDir: 'Download directory',
			submit: 'Add',
			added: 'Queued {name}.',
			addFailed: 'Could not queue the download.',
			empty: 'Nothing queued. Paste a URL above to start.',
			keys: 'Focus a row: {pause} pause or resume, {cancel} cancel, {remove} remove. {focus} focuses the URL.',
			actionFailed: 'Could not change the download.',
			removed: 'Removed {name} from the list. The file stays where it is.'
		},
		state: {
			queued: 'Queued',
			running: 'Downloading',
			paused: 'Paused',
			done: 'Done',
			failed: 'Failed',
			canceled: 'Canceled'
		},
		action: {
			pause: 'Pause',
			resume: 'Resume',
			retry: 'Retry',
			cancel: 'Cancel',
			remove: 'Remove'
		},
		progress: '{done} of {total}',
		progressUnknown: '{done}',
		into: 'into {target}',
		retrying: 'Retry {attempt} of {max} {when}',
		storage: {
			group: 'Storage',
			used: 'Used by downloads',
			usedHint: 'Finished files and partial downloads the manager still tracks.',
			usedValue: '{used} of {max}',
			usedUnlimited: '{used}, no budget',
			free: 'Free on disk',
			freeHint: 'Downloads stop before free space drops under the floor.',
			freeValue: '{free} free, {floor} kept free',
			freeUnknown: 'Unknown on this platform',
			cleanup: 'Clean up',
			cleanupHint: 'Deletes partial files of failed and canceled downloads and records past retention. Never deletes finished files.',
			cleaned: {
				one: 'Removed {count} partial file, freed {freed}.',
				other: 'Removed {count} partial files, freed {freed}.'
			},
			cleanupFailed: 'Cleanup failed.'
		},
		limits: {
			group: 'Limits',
			dir: 'Download directory',
			dirHint: 'Absolute server path for downloads without a library.',
			max: 'Budget (GiB)',
			maxHint: 'Total the downloads may occupy. 0 means no budget.',
			minFree: 'Keep free (GiB)',
			minFreeHint: 'Free space that must remain on the target disk. 0 disables the floor.',
			concurrency: 'Parallel downloads',
			concurrencyHint: 'How many transfers run at once (1–8).',
			keepDays: 'Keep records (days)',
			keepDaysHint: 'Cleanup forgets finished, failed and canceled entries after this. 0 keeps them.',
			retries: 'Automatic retries',
			retriesHint: 'Network errors, 5xx answers and dropped transfers retry with growing waits, resuming by byte range. 0 disables.',
			saved: 'Download limits saved.',
			saveFailed: 'Could not save the download limits.',
			loadFailed: 'Could not load downloads.'
		},
		error: {
			quotaExceeded: 'The download budget is full.',
			diskFull: 'The disk would drop under the free-space floor.'
		}
	},
	reader: {
		info: {
			label: 'From the archive',
			field: {
				series: 'Series',
				number: 'Number',
				volume: 'Volume',
				writer: 'Writer',
				artist: 'Artist',
				publisher: 'Publisher',
				year: 'Year',
				language: 'Language'
			},
			direction: {
				rtl: 'Reads right to left',
				ltr: 'Reads left to right'
			},
			source: {
				archive: 'declared by the archive',
				library: 'library default'
			},
			pages: { one: '{count} page', other: '{count} pages' }
		}
	},
	offline: {
		title: 'Offline — Settings — Lain',
		save: 'Save offline',
		saving: 'Saving {done} of {total}',
		saved: 'Saved offline',
		remove: 'Remove offline copy',
		savedToast: 'Saved {name} for offline reading.',
		removedToast: 'Removed the offline copy of {name}.',
		quota: 'Not enough offline space: needs {need}, {budget} allowed in this browser.',
		unsupported: 'This browser cannot keep items offline (it needs HTTPS or localhost).',
		failed: 'Could not save offline.',
		storage: {
			group: 'This browser',
			used: 'Offline storage',
			usedHint: 'Comics and manga saved here open without the server.',
			usedValue: '{used} of {budget}',
			usedUnlimited: '{used}, no limit',
			cap: 'Limit (GiB)',
			capHint: 'At most this much, and never more than 80% of what the browser grants. 0 uses the browser’s grant only.',
			clear: 'Remove everything',
			clearHint: 'Deletes every offline copy kept in this browser.',
			cleared: 'Offline storage cleared.'
		},
		items: {
			group: 'Saved items',
			empty: 'Nothing saved yet. On a comic or manga page, press {key} or choose Save offline.',
			pages: { one: '{count} page', other: '{count} pages' },
			open: 'Read',
			keys: 'Focus a row: {open} reads it, {remove} removes it.'
		}
	}
} as const;

export default en;
