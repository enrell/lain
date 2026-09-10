import QtQuick
import QtQuick.Layouts
import Lain
import "../components"

// Busca funcional: input grande + resultados ao vivo do catálogo mock.
Rectangle {
    id: root
    anchors.fill: parent
    color: "#000000B8"
    z: 20

    signal openMedia(var media)
    signal closed()

    function openWith(q) {
        visible = true;
        query = q;
        searchInput.text = q;
        searchInput.forceActiveFocus();
    }

    property string query: ""

    readonly property var results: {
        var all = server.movies();
        var q = query.trim().toLowerCase();
        if (q === "")
            return all.slice(0, 8);
        return all.filter(m => String(m.title).toLowerCase().indexOf(q) >= 0
            || String(m.genre).toLowerCase().indexOf(q) >= 0);
    }

    MouseArea {
        anchors.fill: parent
        onClicked: { root.visible = false; root.closed(); }
    }

    Rectangle {
        anchors.centerIn: parent
        width: Math.min(900, parent.width - 120)
        height: Math.min(620, parent.height - 120)
        radius: Tokens.radiusLg
        color: Tokens.bgElevated
        border.color: Tokens.borderSubtle
        MouseArea { anchors.fill: parent } // consome cliques dentro do painel

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 22
            spacing: 14

            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                Text {
                    text: ""
                    color: Tokens.themeAccent
                    font.family: Tokens.fontFamily
                    font.pixelSize: 20
                }
                TextInput {
                    id: searchInput
                    Layout.fillWidth: true
                    color: Tokens.textPrimary
                    font.family: Tokens.fontFamily
                    font.pixelSize: 20
                    onTextChanged: root.query = text
                    onAccepted: { if (root.results.length > 0) root.openMedia(root.results[0]); }
                }
            }

            Text {
                text: query === "" ? "Suggestions" : results.length + (results.length === 1 ? " result" : " results")
                color: Tokens.textTertiary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.metaSize
            }

            GridView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                cellWidth: Tokens.posterWidth + Tokens.cardGap
                cellHeight: Tokens.posterWidth * 1.5 + 52
                boundsBehavior: Flickable.StopAtBounds
                model: root.results
                delegate: PosterCard {
                    media: modelData
                    onOpen: m => root.openMedia(m)
                }
            }
        }
    }
}
