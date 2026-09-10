import QtQuick
import Lain

// Pessoa: círculo + nome + papel. Sem foto: inicial sobre surface.
Rectangle {
    width: 96
    height: 138
    color: "transparent"
    property var person

    Rectangle {
        id: avatar
        width: 76
        height: 76
        radius: 38
        anchors.horizontalCenter: parent.horizontalCenter
        color: Tokens.surface1
        border.color: Tokens.borderSubtle
        Text {
            anchors.centerIn: parent
            text: person && person.name ? person.name.charAt(0) : ""
            color: Tokens.textTertiary
            font.family: Tokens.fontFamily
            font.pixelSize: 28
            font.bold: true
        }
    }
    Text {
        anchors.top: avatar.bottom
        anchors.topMargin: 8
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        text: person ? person.name : ""
        color: Tokens.textPrimary
        font.family: Tokens.fontFamily
        font.pixelSize: Tokens.metaSize
        elide: Text.ElideRight
    }
    Text {
        anchors.top: avatar.bottom
        anchors.topMargin: 26
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        text: person ? person.role : ""
        color: Tokens.textTertiary
        font.family: Tokens.fontFamily
        font.pixelSize: Tokens.metaSize - 1
        elide: Text.ElideRight
    }
}
