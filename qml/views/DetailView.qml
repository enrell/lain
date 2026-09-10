import QtQuick
import QtQuick.Layouts
import Lain
import "../components"

// Detail mock (§24): hero cinematográfico + cast + relacionados +
// extras + ficha técnica. Tudo do server.media(id).
ColumnLayout {
    property var media
    signal playMedia(var media)
    signal openMedia(var media)
    signal back()
    spacing: 0

    Hero {
        Layout.fillWidth: true
        media: parent.media
        onPlay: m => playMedia(m)
        onMoreInfo: m => openMedia(m)
    }

    // Voltar: sobre o hero, discreto
    Rectangle {
        x: Tokens.pageMargin
        y: 20
        z: 5
        width: 104
        height: 36
        radius: Tokens.radiusPill
        color: Qt.alpha(Tokens.bgPrimary, 0.55)
        border.color: Tokens.borderSubtle
        Text {
            anchors.centerIn: parent
            text: "‹  Back"
            color: Tokens.textPrimary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.metaSize
        }
        MouseArea { anchors.fill: parent; onClicked: back() }
    }

    ColumnLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Tokens.pageMargin
        Layout.rightMargin: Tokens.pageMargin
        Layout.topMargin: Tokens.sectionGap
        Layout.bottomMargin: Tokens.sectionGap
        spacing: Tokens.sectionGap

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 12
            Text {
                text: "Cast"
                color: Tokens.textPrimary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.sectionSize
                font.weight: Font.DemiBold
            }
            ListView {
                Layout.fillWidth: true
                Layout.preferredHeight: 140
                orientation: ListView.Horizontal
                spacing: Tokens.cardGap
                boundsBehavior: Flickable.StopAtBounds
                model: parent.parent.media ? parent.parent.media.cast : []
                delegate: PersonCard { person: modelData }
            }
        }

        MediaRow {
            Layout.fillWidth: true
            title: "More Like This"
            rowHeight: Tokens.posterWidth * 1.5 + 52
            model: media ? media.related : []
            delegate: PosterCard { media: modelData; onOpen: m => openMedia(m) }
        }

        MediaRow {
            Layout.fillWidth: true
            title: "Extras"
            rowHeight: Tokens.landscapeWidth * 9 / 16 + 34
            model: media ? media.extras : []
            delegate: LandscapeCard { media: modelData; onOpen: m => openMedia(m) }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 12
            Text {
                text: "Media Info"
                color: Tokens.textPrimary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.sectionSize
                font.weight: Font.DemiBold
            }
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 148
                radius: Tokens.radiusMd
                color: Tokens.surface1
                border.color: Tokens.borderSubtle
                GridLayout {
                    anchors.fill: parent
                    anchors.margins: 20
                    columns: 2
                    columnSpacing: 24
                    rowSpacing: 8
                    Text { text: "Video"; color: Tokens.textTertiary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                    Text { text: media && media.tech ? media.tech.video : ""; color: Tokens.textPrimary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                    Text { text: "Audio"; color: Tokens.textTertiary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                    Text { text: media && media.tech ? media.tech.audio : ""; color: Tokens.textPrimary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                    Text { text: "Container"; color: Tokens.textTertiary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                    Text { text: media && media.tech ? media.tech.container : ""; color: Tokens.textPrimary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                    Text { text: "Director"; color: Tokens.textTertiary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                    Text { text: media ? media.director : ""; color: Tokens.textPrimary; font.family: Tokens.fontFamily; font.pixelSize: Tokens.metaSize }
                }
            }
        }
    }
}
