import QtQuick
import QtQuick.Layouts
import Lain

// Rail nativa desktop: 64px colapsada, expande por CLIQUE no menu (☰).
// Hover nunca muda geometria — só realça o item sob o cursor.
Rectangle {
    id: rail
    width: expanded ? 210 : Tokens.navRailWidth
    color: Tokens.bgSecondary
    border.color: Tokens.borderSubtle
    property bool expanded: false
    property string current: "home"
    signal navigate(string route)

    Behavior on width { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        anchors.topMargin: 14
        anchors.bottomMargin: 12
        spacing: 6

        Repeater {
            model: [
                { route: "home", icon: "\uf015", label: "Home" },
                { route: "movies", icon: "\uf008", label: "Movies" },
                { route: "shows", icon: "\uf26c", label: "Shows" },
                { route: "collections", icon: "\uf009", label: "Collections" }
            ]
            delegate: railItem
        }

        Item { Layout.fillHeight: true; Layout.minimumHeight: 12 }

        Rectangle {
            Layout.fillWidth: true
            Layout.leftMargin: 12
            Layout.rightMargin: 12
            height: 1
            color: Tokens.borderSubtle
        }

        Repeater {
            model: [
                { route: "settings", icon: "\uf013", label: "Settings" }
            ]
            delegate: railItem
        }
    }

    Component {
        id: railItem
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 46
            radius: 10
            color: rail.current === modelData.route ? Tokens.selectedFill : "transparent"
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                spacing: 10
                Text {
                    Layout.preferredWidth: 28
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    text: modelData.icon
                    color: rail.current === modelData.route ? Tokens.themeAccent : Tokens.textTertiary
                    font.family: Tokens.fontFamily
                    font.pixelSize: 17
                }
                Text {
                    Layout.fillWidth: true
                    verticalAlignment: Text.AlignVCenter
                    text: modelData.label
                    color: rail.current === modelData.route ? Tokens.textPrimary : Tokens.textSecondary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.cardTitleSize
                    elide: Text.ElideRight
                    visible: rail.expanded
                }
            }
            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                onEntered: parent.color = rail.current === modelData.route ? Tokens.selectedFill : Tokens.hoverFill
                onExited: parent.color = rail.current === modelData.route ? Tokens.selectedFill : "transparent"
                onPressed: rail.expanded = true
                onClicked: { rail.current = modelData.route; rail.navigate(modelData.route); }
            }
            Accessible.name: modelData.label
        }
    }
}
