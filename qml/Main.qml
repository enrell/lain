import QtQuick
import QtQuick.Layouts
import Lain
import "components"
import "views"

Window {
    id: root
    width: 1280
    height: 800
    visible: true
    title: "lain"
    color: Tokens.bgPrimary

    property var homeData: server.home()
    property var currentMedia: null
    property string route: "home"
    property string returnRoute: "home"

    // Conteúdo com margem fixa da rail colapsada; a rail flutua por cima
    // (z) então expandir nunca desloca o layout — sem jitter de hitbox.
    Flickable {
        id: page
        anchors.fill: parent
        anchors.leftMargin: Tokens.navRailWidth
        contentWidth: width
        contentHeight: content.height
        boundsBehavior: Flickable.StopAtBounds
        ColumnLayout {
            id: content
            width: parent.width
            HomeView {
                Layout.fillWidth: true
                visible: root.route === "home"
                home: root.homeData
                onOpenMedia: m => { root.currentMedia = server.media(m.id); root.returnRoute = "home"; root.route = "detail"; }
                onPlayMedia: m => { root.currentMedia = server.media(m.id); root.route = "player"; }
                onSearchRequested: t => searchOverlay.openWith(t)
                onAccount: root.route = "settings"
            }
            LibraryView {
                Layout.fillWidth: true
                visible: root.route === "movies" || root.route === "shows"
                title: root.route === "shows" ? "Shows" : "Movies"
                items: root.route === "shows"
                    ? (root.homeData ? root.homeData.recentlyAdded : [])
                    : server.movies()
                onOpenMedia: m => { root.currentMedia = server.media(m.id); root.returnRoute = root.route; root.route = "detail"; }
                onSearchRequested: t => searchOverlay.openWith(t)
                onAccount: root.route = "settings"
            }
            DetailView {
                Layout.fillWidth: true
                visible: root.route === "detail"
                media: root.currentMedia
                onOpenMedia: m => { root.currentMedia = server.media(m.id); }
                onPlayMedia: m => { root.currentMedia = server.media(m.id); root.route = "player"; }
                onBack: root.route = root.returnRoute
            }
            PlayerView {
                id: playerView
                visible: root.route === "player"
                media: root.currentMedia
                onToggleFullscreen: root.visibility = root.visibility === Window.FullScreen ? Window.Windowed : Window.FullScreen
                onBack: root.route = root.returnRoute
            }
            SettingsView {
                visible: root.route === "settings"
                Layout.fillWidth: true
                Layout.leftMargin: Tokens.pageMargin
                Layout.rightMargin: Tokens.pageMargin
                Layout.topMargin: 8
                onSearchRequested: t => searchOverlay.openWith(t)
                onAccount: root.route = "settings"
            }
        }
    }

    NavRail {
        id: navRail
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        z: 10
        visible: root.visibility !== Window.FullScreen
        onNavigate: r => { root.route = r; }
    }

    // Tracking de hover por posição (não por eventos de borda): zona
    // invisível que nunca intercepta cliques. Histerese 96/230 elimina
    // flicker — botões da rail voltam a ser só botões.
    MouseArea {
        anchors.fill: parent
        z: 5
        hoverEnabled: true
        acceptedButtons: Qt.NoButton
        onPositionChanged: mouse => {
            if (mouse.x < 96) navRail.expanded = true;
            else if (mouse.x > 230) navRail.expanded = false;
        }
    }

    // Busca funcional: Ctrl+F oficial, / como extra.
    SearchOverlay {
        id: searchOverlay
        visible: false
        onOpenMedia: m => { visible = false; root.currentMedia = server.media(m.id); root.returnRoute = root.route; root.route = "detail"; }
    }

    Shortcut { sequence: "Ctrl+F"; onActivated: searchOverlay.openWith("") }
    Shortcut { sequence: "/"; onActivated: searchOverlay.openWith("") }
    Shortcut { sequence: "Alt+Left"; onActivated: root.route = "home" }
    Shortcut {
        sequence: "Esc"
        onActivated: {
            if (root.visibility === Window.FullScreen)
                root.visibility = Window.Windowed;
            else if (searchOverlay.visible)
                searchOverlay.visible = false;
            else if (root.route === "detail" || root.route === "player")
                root.route = root.returnRoute;
        }
    }

    // Atalhos do player (§31)
    Shortcut { enabled: root.route === "player"; sequence: "Space"; onActivated: playerView.togglePause() }
    Shortcut { enabled: root.route === "player"; sequence: "Left"; onActivated: playerView.seekBy(-10) }
    Shortcut { enabled: root.route === "player"; sequence: "Right"; onActivated: playerView.seekBy(10) }
    Shortcut { enabled: root.route === "player"; sequence: "Up"; onActivated: playerView.adjustVolume(5) }
    Shortcut { enabled: root.route === "player"; sequence: "Down"; onActivated: playerView.adjustVolume(-5) }
    Shortcut { enabled: root.route === "player"; sequence: "M"; onActivated: playerView.toggleMute() }
    Shortcut { enabled: root.route === "player"; sequence: "F"; onActivated: playerView.toggleFullscreen() }
    Shortcut { enabled: root.route === "player"; sequence: "A"; onActivated: playerView.cycleAudio() }
    Shortcut { enabled: root.route === "player"; sequence: "S"; onActivated: playerView.cycleSubtitle() }
}
