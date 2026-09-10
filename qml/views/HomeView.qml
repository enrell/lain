import QtQuick
import QtQuick.Layouts
import Lain
import "../components"

// Home: hero full-bleed + rows alinhadas ao gutter único.
ColumnLayout {
    property var home
    signal openMedia(var media)
    signal playMedia(var media)
    signal searchRequested(string text)
    signal account()
    spacing: Tokens.sectionGap

    PageHeader {
        Layout.topMargin: 8
        eyebrow: "HOME"
        onSearchRequested: t => searchRequested(t)
        onOpenMedia: m => openMedia(m)
        onAccount: account()
    }
    Hero {
        Layout.fillWidth: true
        media: home ? home.hero : null
        onPlay: m => playMedia(m)
        onMoreInfo: m => openMedia(m)
    }
    MediaRow {
        Layout.fillWidth: true
        Layout.leftMargin: Tokens.pageMargin
        Layout.rightMargin: Tokens.pageMargin
        title: "Continue Watching"
        rowHeight: Tokens.landscapeWidth * 9 / 16 + 34
        model: home ? home.continueWatching : []
        delegate: LandscapeCard { media: modelData; onOpen: m => openMedia(m) }
    }
    MediaRow {
        Layout.fillWidth: true
        Layout.leftMargin: Tokens.pageMargin
        Layout.rightMargin: Tokens.pageMargin
        Layout.bottomMargin: Tokens.sectionGap
        title: "Recently Added"
        rowHeight: Tokens.posterWidth * 1.5 + 52
        model: home ? home.recentlyAdded : []
        delegate: PosterCard { media: modelData; onOpen: m => openMedia(m) }
    }
}
