import QtQuick
import QtQuick.Layouts
import Lain
import "../components"

// Player real sobre libmpv: timeline, volume, faixas, Anime4K, OSD.
Item {
    id: playerRoot
    Layout.fillWidth: true
    Layout.preferredHeight: Math.max(620, page.height)

    property var media
    property bool hasStream: false
    property bool controlsVisible: true
    property string osdText: ""
    property real scrubValue: 0

    signal toggleFullscreen()
    signal back()

    function fmt(s) {
        if (!isFinite(s) || s < 0 || s === null)
            return "--:--";
        s = Math.floor(s);
        var h = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60), sec = s % 60;
        var mm = (h > 0 && m < 10 ? "0" : "") + m;
        var ss = (sec < 10 ? "0" : "") + sec;
        return (h > 0 ? h + ":" : "") + mm + ":" + ss;
    }
    function trackLabel(t) {
        var s = t.lang ? t.lang : "";
        if (t.title)
            s = s ? s + " · " + t.title : t.title;
        return s === "" ? "Track " + t.id : s;
    }
    function wake() {
        controlsVisible = true;
        hideTimer.restart();
    }
    function osd(t) {
        osdText = t;
        osdTimer.restart();
    }
    function togglePause() { mpv.togglePause(); wake(); }
    function seekBy(d) { mpv.seekBy(d); wake(); }
    function adjustVolume(d) { mpv.setVolume(mpv.volume + d); osd("Volume " + Math.round(mpv.volume + d)); wake(); }
    function toggleMute() { mpv.setMuted(!mpv.muted); wake(); }
    function cycleAudio() { mpv.cycleAudio(); osd("Audio"); wake(); }
    function cycleSubtitle() { mpv.cycleSubtitle(); osd("Legenda"); wake(); }
    function cycleShader() { mpv.cycleShaderPreset(); osd(mpv.shaderInfo); wake(); }

    MpvItem {
        id: mpv
        anchors.fill: parent
        Component.onCompleted: {
            var u = media ? server.streamUrl(media.id) : "";
            if (u) {
                play(u);
                hasStream = true;
            }
        }
    }

    // Wake em qualquer movimento; clique no vídeo pausa
    MouseArea {
        anchors.fill: parent
        z: 1
        hoverEnabled: true
        acceptedButtons: Qt.NoButton
        onPositionChanged: wake()
    }
    MouseArea {
        anchors.fill: parent
        z: 2
        onClicked: {
            audioPopup.visible = false;
            subPopup.visible = false;
            togglePause();
        }
    }

    Timer {
        id: hideTimer
        interval: 2800
        repeat: false
        onTriggered: { if (!mpv.paused && hasStream) controlsVisible = false; }
    }
    Timer {
        id: osdTimer
        interval: 1300
        repeat: false
        onTriggered: osdText = ""
    }
    Component.onCompleted: wake()

    // OSD central
    Text {
        anchors.centerIn: parent
        z: 3
        text: osdText
        color: "white"
        font.family: Tokens.fontFamily
        font.pixelSize: 17
        font.bold: true
        opacity: osdText === "" ? 0 : 1
        Behavior on opacity { NumberAnimation { duration: 200 } }
    }

    // Sem fonte: overlay de teste
    Rectangle {
        anchors.fill: parent
        z: 4
        color: Qt.alpha(Tokens.bgPrimary, 0.85)
        visible: !hasStream
        ColumnLayout {
            anchors.centerIn: parent
            spacing: 12
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: media ? media.title : "Player"
                color: Tokens.textPrimary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.sectionSize
                font.weight: Font.DemiBold
            }
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: "Sem fonte do backend (mock)"
                color: Tokens.textTertiary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.metaSize
            }
            Rectangle {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 220
                Layout.preferredHeight: Tokens.buttonHeight
                radius: Tokens.radiusMd
                color: Tokens.themeAccent
                Text {
                    anchors.centerIn: parent
                    text: "Test pattern"
                    color: "black"
                    font.family: Tokens.fontFamily
                    font.bold: true
                }
                MouseArea {
                    anchors.fill: parent
                    onClicked: { mpv.playTestPattern(); hasStream = true; wake(); }
                }
            }
            Text {
                Layout.alignment: Qt.AlignHCenter
                visible: mpv.errorText !== ""
                text: mpv.errorText
                color: Tokens.themeUrgent
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.metaSize
            }
        }
    }

    // Barra superior
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 64
        z: 5
        color: "transparent"
        visible: controlsVisible
        gradient: Gradient {
            GradientStop { position: 0.0; color: Qt.alpha(Tokens.bgPrimary, 0.75) }
            GradientStop { position: 1.0; color: Qt.alpha(Tokens.bgPrimary, 0.0) }
        }
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            spacing: 12
            PlayerButton {
                glyph: "\uf053"
                glyphSize: 15
                onPressed: back()
            }
            Text {
                Layout.fillWidth: true
                text: media ? media.title : "Test Pattern"
                color: Tokens.textPrimary
                font.family: Tokens.fontFamily
                font.pixelSize: 16
                elide: Text.ElideRight
            }
            Text {
                text: mpv.shaderInfo
                color: Tokens.textTertiary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.metaSize - 1
            }
        }
    }

    // Controles inferiores
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 108
        z: 5
        color: "transparent"
        visible: controlsVisible
        gradient: Gradient {
            GradientStop { position: 0.0; color: Qt.alpha(Tokens.bgPrimary, 0.0) }
            GradientStop { position: 1.0; color: Qt.alpha(Tokens.bgPrimary, 0.85) }
        }
        ColumnLayout {
            anchors.fill: parent
            anchors.leftMargin: 20
            anchors.rightMargin: 20
            anchors.bottomMargin: 10
            spacing: 2
            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                Text {
                    text: fmt(timelineBar.scrubbing ? playerRoot.scrubValue : mpv.position)
                    color: Tokens.textSecondary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.metaSize
                }
                SeekBar {
                    id: timelineBar
                    Layout.fillWidth: true
                    from: 0
                    to: mpv.duration > 0 ? mpv.duration : 1
                    value: scrubbing ? playerRoot.scrubValue : mpv.position
                    fillColor: media && media.accent ? media.accent : Tokens.themeAccent
                    onScrubbed: v => { playerRoot.scrubValue = v; }
                    onReleased: v => { mpv.seek(v); }
                }
                Text {
                    text: fmt(mpv.duration)
                    color: Tokens.textSecondary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.metaSize
                }
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: 4
                PlayerButton { glyph: "-10s"; glyphSize: 12; bold: true; onPressed: seekBy(-10) }
                PlayerButton { glyph: mpv.paused ? "\uf04b" : "\uf04c"; glyphSize: 18; onPressed: togglePause() }
                PlayerButton { glyph: "+10s"; glyphSize: 12; bold: true; onPressed: seekBy(10) }
                Item { Layout.fillWidth: true }
                PlayerButton { glyph: mpv.muted ? "\uf026" : "\uf028"; glyphSize: 16; onPressed: toggleMute() }
                SeekBar {
                    Layout.preferredWidth: 90
                    from: 0
                    to: 100
                    value: mpv.volume
                    fillColor: Tokens.textPrimary
                    onScrubbed: v => mpv.setVolume(v)
                    onReleased: v => mpv.setVolume(v)
                }
                PlayerButton { glyph: "\uf001"; glyphSize: 15; onPressed: { audioPopup.visible = !audioPopup.visible; subPopup.visible = false; } }
                PlayerButton { glyph: "\uf20a"; glyphSize: 15; onPressed: { subPopup.visible = !subPopup.visible; audioPopup.visible = false; } }
                PlayerButton { glyph: "\uf0d0"; glyphSize: 15; onPressed: cycleShader() }
                PlayerButton { glyph: "\uf065"; glyphSize: 14; onPressed: toggleFullscreen() }
            }
        }
    }

    // Popup de faixas de áudio
    Rectangle {
        id: audioPopup
        visible: false
        z: 6
        anchors.right: parent.right
        anchors.rightMargin: 20
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 118
        width: 280
        height: Math.min(300, trackList.implicitHeight + 28)
        radius: Tokens.radiusMd
        color: Tokens.bgElevated
        border.color: Tokens.borderSubtle
        Column {
            id: trackList
            anchors.fill: parent
            anchors.margins: 14
            spacing: 4
            Text { text: "Audio"; color: Tokens.textTertiary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
            Repeater {
                model: mpv.audioTracks
                delegate: Text {
                    width: trackList.width
                    text: (mpv.audioId === modelData.id ? "● " : "○ ") + trackLabel(modelData)
                    color: mpv.audioId === modelData.id ? Tokens.themeAccent : Tokens.textPrimary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.metaSize
                    elide: Text.ElideRight
                    MouseArea {
                        anchors.fill: parent
                        onClicked: { mpv.setAudioTrack(modelData.id); audioPopup.visible = false; }
                    }
                }
            }
        }
    }

    // Popup de legendas
    Rectangle {
        id: subPopup
        visible: false
        z: 6
        anchors.right: parent.right
        anchors.rightMargin: 20
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 118
        width: 280
        height: Math.min(300, subList.implicitHeight + 28)
        radius: Tokens.radiusMd
        color: Tokens.bgElevated
        border.color: Tokens.borderSubtle
        Column {
            id: subList
            anchors.fill: parent
            anchors.margins: 14
            spacing: 4
            Text { text: "Subtitles"; color: Tokens.textTertiary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
            Text {
                width: subList.width
                text: (mpv.subtitleId < 0 ? "● " : "○ ") + "Off"
                color: mpv.subtitleId < 0 ? Tokens.themeAccent : Tokens.textPrimary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.metaSize
                MouseArea {
                    anchors.fill: parent
                    onClicked: { mpv.setSubtitleTrack(-1); subPopup.visible = false; }
                }
            }
            Repeater {
                model: mpv.subtitleTracks
                delegate: Text {
                    width: subList.width
                    text: (mpv.subtitleId === modelData.id ? "● " : "○ ") + trackLabel(modelData)
                    color: mpv.subtitleId === modelData.id ? Tokens.themeAccent : Tokens.textPrimary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.metaSize
                    elide: Text.ElideRight
                    MouseArea {
                        anchors.fill: parent
                        onClicked: { mpv.setSubtitleTrack(modelData.id); subPopup.visible = false; }
                    }
                }
            }
        }
    }
}
