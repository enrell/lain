import QtQuick
import QtQuick.Layouts
import Lain
import "../components"

// Library (§19-21): exploração eficiente. Grid denso, filtros por
// gênero + ordenação funcionais sobre o model, abre Detail no clique.
ColumnLayout {
    property string title: "Movies"
    property var items: []
    signal openMedia(var media)

    property string selectedGenre: "All"
    property string sortMode: "title" // title | year | rating
    signal searchRequested(string text)
    signal account()

    readonly property var genres: ["All", "Sci-Fi", "Crime", "Horror", "Animation"]

    function computeShown() {
        var list = (items || []).slice();
        if (selectedGenre !== "All")
            list = list.filter(m => m.genre === selectedGenre);
        if (sortMode === "title")
            list.sort((a, b) => String(a.title).localeCompare(String(b.title)));
        else if (sortMode === "year")
            list.sort((a, b) => Number(b.year) - Number(a.year));
        else if (sortMode === "rating")
            list.sort((a, b) => Number(b.rating) - Number(a.rating));
        return list;
    }
    readonly property var shown: computeShown()

    spacing: 0

    PageHeader {
        Layout.topMargin: 8
        eyebrow: "LIBRARY"
        onSearchRequested: t => searchRequested(t)
        onOpenMedia: m => openMedia(m)
        onAccount: account()
    }

    // Header: título + contagem
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Tokens.pageMargin
        Layout.rightMargin: Tokens.pageMargin
        Layout.topMargin: 8
        spacing: 12
        Text {
            text: title
            color: Tokens.textPrimary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.pageTitleSize
            font.weight: Font.DemiBold
        }
        Text {
            Layout.alignment: Qt.AlignBottom
            Layout.bottomMargin: 4
            text: shown.length + (shown.length === 1 ? " title" : " titles")
            color: Tokens.textTertiary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.metaSize
        }
    }

    // Filtros + ordenação
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Tokens.pageMargin
        Layout.rightMargin: Tokens.pageMargin
        Layout.topMargin: 18
        spacing: 8

        Repeater {
            model: genres
            delegate: Rectangle {
                Layout.preferredHeight: 32
                Layout.preferredWidth: chipLabel.implicitWidth + 28
                radius: Tokens.radiusPill
                color: selectedGenre === modelData ? Qt.tint(Tokens.bgPrimary, Qt.alpha(Tokens.themeAccent, 0.16)) : Tokens.surface1
                border.color: selectedGenre === modelData ? Qt.alpha(Tokens.themeAccent, 0.5) : Tokens.borderSubtle
                Text {
                    id: chipLabel
                    anchors.centerIn: parent
                    text: modelData
                    color: selectedGenre === modelData ? Tokens.themeAccent : Tokens.textSecondary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.metaSize
                }
                MouseArea {
                    anchors.fill: parent
                    onClicked: selectedGenre = modelData
                }
            }
        }

        Item { Layout.fillWidth: true }

        Text {
            text: "Sort"
            color: Tokens.textTertiary
            font.family: Tokens.fontFamily
            font.pixelSize: Tokens.metaSize
        }
        Repeater {
            model: [
                { key: "title", label: "Title" },
                { key: "year", label: "Year" },
                { key: "rating", label: "Rating" }
            ]
            delegate: Text {
                text: modelData.label
                color: sortMode === modelData.key ? Tokens.themeAccent : Tokens.textSecondary
                font.family: Tokens.fontFamily
                font.pixelSize: Tokens.metaSize
                font.bold: sortMode === modelData.key
                MouseArea {
                    anchors.fill: parent
                    onClicked: sortMode = modelData.key
                }
            }
        }
    }

    // Grid denso desktop
    GridView {
        id: grid
        Layout.fillWidth: true
        Layout.leftMargin: Tokens.pageMargin
        Layout.rightMargin: Tokens.pageMargin
        Layout.topMargin: 20
        Layout.bottomMargin: Tokens.sectionGap
        Layout.preferredHeight: Math.max(320, contentHeight)
        cellWidth: Tokens.posterWidth + Tokens.cardGap
        cellHeight: Tokens.posterWidth * 1.5 + 56
        boundsBehavior: Flickable.StopAtBounds
        model: shown
        delegate: PosterCard {
            media: modelData
            highlighted: GridView.isCurrentItem
            onOpen: m => openMedia(m)
        }
    }
}
