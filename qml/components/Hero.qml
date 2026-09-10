import QtQuick
import QtQuick.Layouts
import Lain
import "../theme/Color.js" as Color

// Hero: backdrop = wash da cor da mídia + gradientes de legibilidade.
// Conteúdo alinhado ao gutter único (Tokens.pageMargin).
Rectangle {
    id: hero
    height: 480
    color: Tokens.bgPrimary
    property var media
    property string accent: media && media.accent ? media.accent : "#8A93A3"
    signal play(var media)
    signal moreInfo(var media)

    // Wash procedural da mídia (placeholder até chegar artwork real)
    Rectangle {
        anchors.fill: parent
        color: Color.shade(hero.accent, 0.24)
        Behavior on color { ColorAnimation { duration: 400 } }
    }
    // Legibilidade à esquerda (texto) — horizontal
    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            orientation: Gradient.Horizontal
            GradientStop { position: 0.0; color: Qt.alpha(Tokens.bgPrimary, 0.95) }
            GradientStop { position: 0.45; color: Qt.alpha(Tokens.bgPrimary, 0.45) }
            GradientStop { position: 0.75; color: Qt.alpha(Tokens.bgPrimary, 0.0) }
        }
    }
    // Assentamento na base (transição p/ o resto da página) — vertical
    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            GradientStop { position: 0.45; color: Qt.alpha(Tokens.bgPrimary, 0.0) }
            GradientStop { position: 1.0; color: Tokens.bgPrimary }
        }
    }

    ColumnLayout {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.leftMargin: Tokens.pageMargin
        anchors.rightMargin: Tokens.pageMargin
        anchors.bottomMargin: 36
        spacing: 0

        // Marca de accent: mesmo gesto das progress bars
        Rectangle {
            width: 28; height: 3; radius: 2
            color: hero.accent
        }
        Text {
            Layout.topMargin: 14
            Layout.fillWidth: true
            text: media ? media.title : ""
            color: Tokens.textPrimary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.heroSize
            font.bold: true
            font.capitalization: Font.AllUppercase
            font.letterSpacing: 2
            lineHeight: 0.95
            elide: Text.ElideRight
        }
        Text {
            Layout.topMargin: 10
            text: media ? media.year + "  ·  " + media.runtime + "  ·  ★ " + Number(media.rating).toFixed(1) + "  ·  " + media.quality : ""
            color: Tokens.textSecondary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.metaSize + 1
            font.letterSpacing: 0.4
        }
        Text {
            Layout.topMargin: 10
            Layout.maximumWidth: 640
            text: media ? media.overview : ""
            color: Tokens.textSecondary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.bodySize
            lineHeight: 1.5
            wrapMode: Text.WordWrap
            maximumLineCount: 2
            elide: Text.ElideRight
        }
        RowLayout {
            Layout.topMargin: 18
            spacing: 10
            Rectangle {
                Layout.preferredWidth: 136
                Layout.preferredHeight: Tokens.buttonHeight
                radius: Tokens.radiusMd
                color: hero.accent
                Text {
                    anchors.centerIn: parent
                    text: "▶  Play"
                    color: "black"
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.bodySize
                    font.bold: true
                }
                MouseArea { anchors.fill: parent; onClicked: hero.play(hero.media) }
            }
            Rectangle {
                Layout.preferredWidth: 136
                Layout.preferredHeight: Tokens.buttonHeight
                radius: Tokens.radiusMd
                color: Tokens.surface2
                Text {
                    anchors.centerIn: parent
                    text: "More Info"
                    color: Tokens.textPrimary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.bodySize
                }
                MouseArea { anchors.fill: parent; onClicked: hero.moreInfo(hero.media) }
            }
        }
    }
}
