import QtQuick
import Lain
import "../theme/Color.js" as Color

// Landscape 16:9 p/ Continue Watching, com progress bar no accent da mídia.
Rectangle {
    id: card
    width: Tokens.landscapeWidth
    height: Tokens.landscapeWidth * 9 / 16 + 30
    color: "transparent"
    property var media
    property string accent: media && media.accent ? media.accent : "#8A93A3"
    signal open(var media)

    Rectangle {
        id: art
        width: parent.width
        height: parent.width * 9 / 16
        radius: Tokens.radiusMd
        gradient: Gradient {
            GradientStop { position: 0.0; color: Color.shade(card.accent, 0.34) }
            GradientStop { position: 1.0; color: Color.shade(card.accent, 0.14) }
        }
        Text {
            anchors.centerIn: parent
            text: card.media ? card.media.title.charAt(0) : ""
            color: "white"
            opacity: 0.07
            font.family: Tokens.fontFamily
            font.pixelSize: 64
            font.bold: true
        }
        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 10
            height: 3
            radius: 2
            color: Tokens.track
            Rectangle {
                width: parent.width * (card.media ? card.media.progress : 0)
                height: parent.height
                radius: 2
                color: card.accent
            }
        }
    }
    MouseArea { anchors.fill: parent; onClicked: card.open(card.media) }
    Text {
        anchors.top: art.bottom
        anchors.topMargin: 8
        width: card.width
        text: card.media ? card.media.title + "  ·  " + Math.round((1 - card.media.progress) * 155) + " min restantes" : ""
        color: Tokens.textSecondary
        font.family: Tokens.fontFamily
        font.pixelSize: Tokens.metaSize
        elide: Text.ElideRight
    }
}
