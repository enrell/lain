<script lang="ts">
	/*
	 * Privacy (docs/slices/social.md, S-5): who sees the profile, activity
	 * and ratings, whether the account is findable and can be asked to
	 * be friends, and the favorites shown on the profile. A YOU page:
	 * every change applies at once (D-087).
	 */
	import { onMount } from 'svelte';
	import { api, type SettingsPatch, type SocialSettings, type Visibility, targetOf } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import { buttonClasses } from '$lib/components/primitives/button-styles';
	import Select from '$lib/components/primitives/Select.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import WorkLink from '$lib/components/social/WorkLink.svelte';
	import { SavedFlash } from '$lib/settings/flash.svelte';
	import { socialError, workKey } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';

	let st = $state<SocialSettings | null>(null);
	const flash = new SavedFlash();

	const levels = $derived(
		(['public', 'friends', 'private'] as Visibility[]).map((v) => ({ value: v, label: t(`social.visibility.${v}`) }))
	);
	const requestOptions = $derived([
		{ value: 'everyone', label: t('social.privacy.requestsEveryone') },
		{ value: 'nobody', label: t('social.privacy.requestsNobody') }
	]);

	onMount(async () => {
		try {
			st = await api.social.settings();
		} catch (err) {
			toasts.error(socialError(err, t('social.privacy.loadFailed')));
		}
	});

	async function patch(row: string, p: SettingsPatch): Promise<boolean> {
		try {
			st = await api.social.updateSettings(p);
			flash.mark(row);
			return true;
		} catch (err) {
			toasts.error(socialError(err, t('social.privacy.saveFailed')));
			return false;
		}
	}

	// One click takes a title off the profile; the toast puts it back where it was (Ctrl+Z).
	async function removeFavorite(key: string): Promise<void> {
		if (!st) return;
		const before = st.favorites.map(targetOf);
		const gone = st.favorites.find((f) => workKey(f) === key);
		if (!(await patch('favorites', { favorites: st.favorites.filter((f) => workKey(f) !== key).map(targetOf) }))) return;
		toasts.undoable(t('social.privacy.favoriteRemoved', { title: gone?.title ?? '' }), () => void patch('favorites', { favorites: before }));
	}
</script>

<svelte:head><title>{t('social.privacy.pageTitle')}</title></svelte:head>

{#if st}
	<SettingsGroup id="visibility" title={t('social.privacy.visibilityGroup')} description={t('settings.groups.privacy.visibility')}>
		<SettingRow id="privacy-profile" label={t('social.privacy.profile')} hint={t('social.privacy.profileHint')} saved={flash.is('profile')}>
			<div class="w-full"><Select value={st.profile} options={levels} aria-label={t('social.privacy.profile')} onValueChange={(v) => void patch('profile', { profile: v as Visibility })} /></div>
		</SettingRow>
		<SettingRow id="privacy-activity" label={t('social.privacy.activity')} hint={t('social.privacy.activityHint')} saved={flash.is('activity')}>
			<div class="w-full"><Select value={st.activity} options={levels} aria-label={t('social.privacy.activity')} onValueChange={(v) => void patch('activity', { activity: v as Visibility })} /></div>
		</SettingRow>
		<SettingRow id="privacy-progress" label={t('social.privacy.hideProgress')} hint={t('social.privacy.hideProgressHint')} saved={flash.is('progress')}>
			<Switch bare label={t('social.privacy.hideProgress')} checked={st.hide_progress} onCheckedChange={(v) => void patch('progress', { hide_progress: v })} />
		</SettingRow>
		<SettingRow id="privacy-ratings" label={t('social.privacy.ratings')} hint={t('social.privacy.ratingsHint')} saved={flash.is('ratings')}>
			<div class="w-full"><Select value={st.ratings} options={levels} aria-label={t('social.privacy.ratings')} onValueChange={(v) => void patch('ratings', { ratings: v as Visibility })} /></div>
		</SettingRow>
	</SettingsGroup>

	<SettingsGroup id="people" title={t('social.privacy.peopleGroup')} description={t('settings.groups.privacy.people')}>
		<SettingRow id="privacy-discoverable" label={t('social.privacy.discoverable')} hint={t('social.privacy.discoverableHint')} saved={flash.is('discoverable')}>
			<Switch bare label={t('social.privacy.discoverable')} checked={st.discoverable} onCheckedChange={(v) => void patch('discoverable', { discoverable: v })} />
		</SettingRow>
		<SettingRow id="privacy-requests" label={t('social.privacy.requests')} hint={t('social.privacy.requestsHint')} saved={flash.is('requests')}>
			<div class="w-full"><Select value={st.allow_requests} options={requestOptions} aria-label={t('social.privacy.requests')} onValueChange={(v) => void patch('requests', { allow_requests: v as 'everyone' | 'nobody' })} /></div>
		</SettingRow>
		<SettingRow id="privacy-blocked" label={t('social.privacy.blocked')} hint={t('social.privacy.blockedHint')}>
			<a href="/social?tab=friends" class={buttonClasses('tonal', 'field')}>{t('social.privacy.manage')}</a>
		</SettingRow>
	</SettingsGroup>

	<SettingsGroup id="favorites" title={t('social.privacy.favoritesGroup')} description={t('settings.groups.privacy.favorites')}>
		<SettingRow id="privacy-favorites" label={t('social.privacy.favorites')} hint={t('social.privacy.favoritesHint')} saved={flash.is('favorites')} stack>
			{#snippet below()}
				<ul class="divide-y divide-hairline">
					{#each st!.favorites as f (workKey(f))}
						<li class="flex items-center justify-between gap-3 py-2.5">
							<WorkLink work={f} />
							<Button variant="ghost" size="field" class="hover:text-danger!" onclick={() => void removeFavorite(workKey(f))} aria-label={t('social.privacy.removeFavoriteNamed', { title: f.title })}>{t('social.privacy.removeFavorite')}</Button>
						</li>
					{:else}
						<li class="py-2.5 text-sm text-muted">{t('social.privacy.noFavorites')}</li>
					{/each}
				</ul>
			{/snippet}
		</SettingRow>
		<SettingRow id="privacy-public-profile" label={t('social.privacy.viewProfile')} hint={t('social.privacy.viewProfileHint')}>
			<a href="/u/{encodeURIComponent(session.user?.id ?? '')}" class={buttonClasses('tonal', 'field')}>{t('social.privacy.open')}</a>
		</SettingRow>
	</SettingsGroup>
{/if}
