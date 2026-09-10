import QtQuick
import Lain

// Botão do player: glifo FA centralizado, pill no hover.
Rectangle {
    implicitWidth: 44
    implicitHeight: 40
    radius: 10
    color: "transparent"

    property string glyph: ""
    property int glyphSize: 13
    property bool bold: false
    signal pressed()

    Text {
        anchors.centerIn: parent
        text: glyph
        color: Tokens.textPrimary
        font.family: Tokens.fontFamily
        font.pixelSize: glyphSize
        font.bold: bold
    }
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        onEntered: parent.color = Tokens.hoverFill
        onExited: parent.color = "transparent"
        onClicked: pressed()
    }
}
