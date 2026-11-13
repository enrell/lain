//! What each capability the server knows is for, in one plain sentence
//! (D-285): `GET /api/plugins` answers it beside each binding, so an
//! operator surface names the job before its id. A capability this
//! table does not know (one a component brought) is described by the
//! providers bound to it ([`crate::Provider::describe`]), or not at all.

/// The sentence for a capability id (`lain.catalog.read@1`), matched on
/// the id without its version first, so a new version of a known job
/// keeps its sentence; `""` when the server does not know it.
pub fn summary(capability: &str) -> &'static str {
    if let Some(s) = by_id(capability) {
        return s;
    }
    let base = capability.rsplit_once('@').map_or(capability, |(b, _)| b);
    by_id(base).unwrap_or("")
}

fn by_id(id: &str) -> Option<&'static str> {
    Some(match id {
        "lain.acquire.extract" => "Turns a link (a page or a file host) into files the server can download.",
        "lain.backup.create" => "Writes the backup archive of the database and the data folder's documents.",
        "lain.catalog.read" => "Serves the catalog to the apps: lists, rails, search and item pages.",
        "lain.catalog.write" => "Stores scanned items in the catalog and marks the ones that are gone.",
        "lain.comic.pages" => "Opens comic and manga archives and serves their pages to the reader.",
        "lain.indexer" => "Searches the indexers you added for releases to grab.",
        "lain.ingest.scan" => "Walks every library folder and updates the catalog.",
        "lain.list.account" => "Keeps the tracking-list accounts you linked (AniList and others).",
        "lain.list.read" => "Reads your tracking lists: what you watch, plan and finished.",
        "lain.list.write" => "Changes entries on your tracking lists.",
        "lain.listlink" => "Talks to a tracking site: imports your list and reports what you finish.",
        "lain.media.identify" => "Turns a file name and its folders into a title, season and episode.",
        "lain.media.probe" => "Reads a file's streams: codecs, tracks, resolution and length.",
        "lain.metadata.enrich" => "Looks titles up after a scan and keeps the posters, synopses and years found.",
        "lain.metadata.resolve" => "Fetches the poster, synopsis and year of a matched title.",
        "lain.metadata.search" => "Searches an external source for the title a file belongs to.",
        "lain.playback.local" => "Plays a file in a player on the server's own machine (mpv or VLC).",
        "lain.playback.plan" => "Decides whether a file plays as it is or has to be converted first.",
        "lain.playback.transcode" => "Converts a file a device cannot play and streams the result.",
        "lain.release.parse" => "Reads a release name: title, season, episode, quality and group.",
        "lain.search.query" => "Answers searches over this server's catalog.",
        "lain.settings.integrations" => "Stores the keys and settings of outside services (lists, subtitles).",
        "lain.settings.transcode" => "Stores the transcoding settings on the Transcoding page.",
        "lain.social.activity" => "Records what friends watch and finish, for the activity feed.",
        "lain.social.collections" => "Keeps collections: hand-picked lists of titles, private or shared.",
        "lain.social.graph" => "Keeps friends, follows, blocks, privacy choices and notifications.",
        "lain.social.reviews" => "Keeps ratings and comments on titles.",
        "lain.source.enumerate" => "Lists the media files in a library folder.",
        "lain.source.watch" => "Notices new and removed files in a library and rescans it.",
        "lain.subtitle" => "Searches subtitle sites for a file's subtitles and downloads them.",
        "lain.subtitle.translate" => "Translates a subtitle track into another language.",
        "lain.transform.thumbnail" => "Cuts the still frames used as thumbnails.",
        "lain.ui.theme" => "Gives the apps their colours (the desktop theme, when there is one).",
        "lain.userstate.progress" => "Remembers where each account stopped in every file.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn known_jobs_have_a_sentence_and_versions_share_it() {
        assert!(summary("lain.catalog.read@1").starts_with("Serves the catalog"));
        assert_eq!(summary("lain.playback.transcode@3"), summary("lain.playback.transcode@1"));
        assert_eq!(summary("lain.example.unknown@1"), "");
        assert_eq!(summary(""), "");
    }
}
