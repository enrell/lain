import QtQuick
import QtQuick.Layouts
import Lain
import "../components"

// Settings stub: prova que o tema é ao vivo (swatches do Omarchy).
ColumnLayout {
    spacing: 16

    signal searchRequested(string text)
    signal account()

    PageHeader {
        eyebrow: "SYSTEM"
        onSearchRequested: t => searchRequested(t)
        onAccount: account()
    }
    Text {
        text: "Settings"
        color: Tokens.textPrimary
        font.family: Tokens.fontFamily
        font.pixelSize: Tokens.pageTitleSize
        font.weight: Font.DemiBold
    }
    Text {
        text: "Appearance follows the Omarchy theme — live, no restart."
        color: Tokens.textSecondary
        font.family: Tokens.fontFamily
        font.pixelSize: Tokens.bodySize
    }
    RowLayout {
        spacing: 12
        Repeater {
            model: [
                { name: "background", c: omarchy.background },
                { name: "foreground", c: omarchy.foreground },
                { name: "accent", c: omarchy.accent },
                { name: "urgent", c: omarchy.urgent },
                { name: "muted", c: omarchy.muted }
            ]
            delegate: ColumnLayout {
                spacing: 6
                Rectangle {
                    Layout.preferredWidth: 72
                    Layout.preferredHeight: 72
                    radius: Tokens.radiusMd
                    color: modelData.c
                    border.color: Tokens.borderSubtle
                }
                Text {
                    text: modelData.name
                    color: Tokens.textTertiary
                    font.family: Tokens.fontFamily
                    font.pixelSize: Tokens.metaSize - 1
                }
            }
        }
    }
    Text {
        text: "Font: monospace  ·  Corners mirror Hyprland rounding (" + omarchy.cornerRadius + ")"
        color: Tokens.textTertiary
        font.family: Tokens.fontFamily
        font.pixelSize: Tokens.metaSize
    }
    Text {
        text: "Build " + buildTs
        color: Tokens.textTertiary
        font.family: Tokens.fontFamily
        font.pixelSize: Tokens.metaSize - 1
    }
}
