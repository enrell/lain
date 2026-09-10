import QtQuick
import QtQuick.Layouts
import Lain
import "../theme/Color.js" as Color

// Header de página: eyebrow à esquerda, busca central com dropdown
// ao vivo, avatar à direita.
Rectangle {
    color: "transparent"
    Layout.preferredHeight: 68
    Layout.fillWidth: true

    property string eyebrow: ""
    property string title: ""
    signal searchRequested(string text)
    signal openMedia(var media)
    signal account()

    function focusSearch() { searchInput.forceActiveFocus(); }

    readonly property var matches: {
        var q = searchInput.text.trim().toLowerCase();
        if (q === "")
            return [];
        var all = server.movies();
        return all.filter(m => String(m.title).toLowerCase().indexOf(q) >= 0
            || String(m.genre).toLowerCase().indexOf(q) >= 0).slice(0, 6);
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Tokens.pageMargin
        anchors.rightMargin: Tokens.pageMargin
        spacing: 16

        ColumnLayout {
            spacing: 2
            Text {
                text: eyebrow
                color: Tokens.textTertiary
                font.family: Tokens.fontFamily
                font.pixelSize: 11
                font.weight: Font.DemiBold
                font.letterSpacing: 2
            }
            Text {
                visible: title !== ""
                text: title
                color: Tokens.textPrimary
                font.family: Tokens.fontFamily
                font.pixelSize: 22
                font.weight: Font.DemiBold
            }
        }

        Item { Layout.fillWidth: true }

        // Campo + dropdown ancorado (container de largura fixa)
        Item {
            Layout.preferredWidth: 480
            Layout.maximumWidth: 560
            Layout.preferredHeight: Tokens.buttonHeight

            Rectangle {
                id: searchBox
                anchors.fill: parent
                radius: Tokens.radiusMd
                color: Tokens.surface1
                border.color: searchInput.activeFocus ? Qt.alpha(Tokens.themeAccent, 0.5) : Tokens.borderSubtle
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14
                    spacing: 10
                    Text {
                        text: "\uf002"
                        color: Tokens.textTertiary
                        font.family: Tokens.fontFamily
                        font.pixelSize: 15
                    }
                    TextInput {
                        id: searchInput
                        Layout.fillWidth: true
                        verticalAlignment: TextInput.AlignVCenter
                        color: Tokens.textPrimary
                        font.family: Tokens.fontFamily
                        font.pixelSize: Tokens.bodySize
                        Text {
                            anchors.fill: parent
                            verticalAlignment: Text.AlignVCenter
                            text: "Search library…"
                            color: Tokens.textTertiary
                            font: searchInput.font
                            visible: searchInput.text === ""
                        }
                        onAccepted: {
                            searchRequested(text);
                            focus = false;
                        }
                        Keys.onEscapePressed: {
                            text = "";
                            focus = false;
                            event.accepted = true;
                        }
                    }
                }
            }

            // Dropdown de resultados ao vivo
            Rectangle {
                id: dropdown
                anchors.top: searchBox.bottom
                anchors.topMargin: 8
                width: searchBox.width
                height: Math.min(6 * 68 + 44, dropCol.implicitHeight + 44)
                z: 50
                radius: Tokens.radiusMd
                color: Tokens.bgElevated
                border.color: Tokens.borderSubtle
                visible: (searchInput.activeFocus && searchInput.text.trim() !== "") || dropdownHover.containsMouse
                MouseArea {
                    id: dropdownHover
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.NoButton
                }
                Column {
                    id: dropCol
                    anchors.fill: parent
                    anchors.margins: 8
                    spacing: 2
                    Repeater {
                        model: matches
                        delegate: Rectangle {
                            width: dropCol.width
                            height: 68
                            radius: 8
                            color: "transparent"
                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: 6
                                spacing: 12
                                Rectangle {
                                    Layout.preferredWidth: 40
                                    Layout.preferredHeight: 56
                                    radius: 6
                                    gradient: Gradient {
                                        GradientStop { position: 0.0; color: Color.shade(modelData.accent, 0.38) }
                                        GradientStop { position: 1.0; color: Color.shade(modelData.accent, 0.15) }
                                    }
                                    Text {
                                        anchors.centerIn: parent
                                        text: String(modelData.title).charAt(0)
                                        color: "white"
                                        opacity: 0.25
                                        font.family: Tokens.fontFamily
                                        font.pixelSize: 22
                                        font.bold: true
                                    }
                                }
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 2
                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData.title
                                        color: Tokens.textPrimary
                                        font.family: Tokens.fontFamily
                                        font.pixelSize: Tokens.cardTitleSize
                                        elide: Text.ElideRight
                                    }
                                    Text {
                                        text: modelData.year + " · " + modelData.genre + " · ★ " + Number(modelData.rating).toFixed(1)
                                        color: Tokens.textTertiary
                                        font.family: Tokens.fontFamily
                                        font.pixelSize: Tokens.metaSize - 1
                                    }
                                }
                            }
                            MouseArea {
                                anchors.fill: parent
                                hoverEnabled: true
                                onEntered: parent.color = Tokens.hoverFill
                                onExited: parent.color = "transparent"
                                onPressed: mouse => {
                                    if (mouse.button === Qt.LeftButton)
                                        openMedia(modelData);
                                }
                            }
                        }
                    }
                    Text {
                        width: dropCol.width
                        visible: matches.length === 0 && searchInput.text.trim() !== ""
                        text: "Nenhum resultado para '" + searchInput.text.trim() + "'"
                        color: Tokens.textTertiary
                        font.family: Tokens.fontFamily
                        font.pixelSize: Tokens.metaSize
                    }
                    Rectangle {
                        width: dropCol.width
                        height: 34
                        radius: 8
                        color: "transparent"
                        visible: matches.length > 0
                        Text {
                            anchors.centerIn: parent
                            text: "Ver todos no overlay  ⏎"
                            color: Tokens.themeAccent
                            font.family: Tokens.fontFamily
                            font.pixelSize: Tokens.metaSize
                        }
                        MouseArea {
                            anchors.fill: parent
                            onPressed: mouse => {
                                if (mouse.button === Qt.LeftButton)
                                    searchRequested(searchInput.text);
                            }
                        }
                    }
                }
            }
        }

        Item { Layout.fillWidth: true }

        Rectangle {
            Layout.preferredWidth: 36
            Layout.preferredHeight: 36
            radius: 18
            color: Qt.tint(Tokens.bgPrimary, Qt.alpha(Tokens.themeAccent, 0.2))
            border.color: Qt.alpha(Tokens.themeAccent, 0.4)
            Text {
                anchors.centerIn: parent
                text: "L"
                color: Tokens.themeAccent
                font.family: Tokens.fontFamily
                font.pixelSize: 15
                font.bold: true
            }
            MouseArea { anchors.fill: parent; onClicked: account() }
        }
    }
}
