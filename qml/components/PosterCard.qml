import QtQuick
import Lain
import QtQuick.Layouts
import "../theme/Color.js" as Color

// Poster 2:3. Normal: arte + título. Metadata só no hover/focus.
Rectangle {
    id: card
    width: Tokens.posterWidth
    height: Tokens.posterWidth * 1.5 + 46
    color: "transparent"
    property var media
    property string accent: media && media.accent ? media.accent : "#8A93A3"
    property bool highlighted: false
    signal open(var media)
    signal play(var media)

    Rectangle {
        id: art
        width: parent.width
        height: parent.width * 1.5
        radius: Tokens.radiusMd
        scale: (card.highlighted || hover.containsMouse) ? 1.03 : 1.0
        Behavior on scale { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
        gradient: Gradient {
            GradientStop { position: 0.0; color: Color.shade(card.accent, 0.38) }
            GradientStop { position: 1.0; color: Color.shade(card.accent, 0.15) }
        }
        // Marca d'água: inicial do título, textura em vez de caixa chapada
        Text {
            anchors.centerIn: parent
            text: card.media ? card.media.title.charAt(0) : ""
            color: "white"
            opacity: 0.08
            font.family: Tokens.fontFamily
            font.pixelSize: 84
            font.bold: true
        }
        Text {
            anchors.centerIn: parent
            text: "▶"
            font.pixelSize: 30
            color: "white"
            visible: card.highlighted || hover.containsMouse
        }
    }
    MouseArea {
        id: hover
        anchors.fill: parent
        hoverEnabled: true
        onClicked: card.open(card.media)
    }
    Column {
        anchors.top: art.bottom
        anchors.topMargin: 8
        width: card.width
        spacing: 2
        Text {
            width: parent.width
            text: card.media ? card.media.title : ""
            color: Tokens.textPrimary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.cardTitleSize
            elide: Text.ElideRight
        }
        Text {
            width: parent.width
            text: card.media ? card.media.year + " · " + card.media.quality : ""
            color: Tokens.textTertiary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.metaSize - 1
            elide: Text.ElideRight
            visible: card.highlighted || hover.containsMouse
        }
    }
}
