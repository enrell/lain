pragma Singleton
import QtQuick

// Tokens derivados do tema Omarchy ao vivo (via `omarchy`).
// Superfícies são compositing sobre themeBg: funcionam em temas
// dark e light sem hardcode. Radius espelha decoration:rounding.
QtObject {
    readonly property color themeBg: omarchy.background
    readonly property color themeFg: omarchy.foreground
    readonly property color themeAccent: omarchy.accent
    readonly property color themeUrgent: omarchy.urgent
    readonly property color themeMuted: omarchy.muted

    readonly property color bgPrimary: themeBg
    readonly property color bgSecondary: Qt.tint(themeBg, Qt.alpha(themeFg, 0.03))
    readonly property color bgElevated: Qt.tint(themeBg, Qt.alpha(themeFg, 0.06))

    readonly property color surface1: Qt.tint(themeBg, Qt.alpha(themeFg, 0.05))
    readonly property color surface2: Qt.tint(themeBg, Qt.alpha(themeFg, 0.09))
    readonly property color track: Qt.tint(themeBg, Qt.alpha(themeFg, 0.14))
    readonly property color borderSubtle: Qt.alpha(themeFg, 0.09)

    // Vocabulário de estados do shell (normal/hover/selected)
    readonly property color hoverFill: Qt.tint(themeBg, Qt.alpha(themeFg, 0.07))
    readonly property color selectedFill: Qt.tint(themeBg, Qt.alpha(themeFg, 0.14))

    readonly property color textPrimary: themeFg
    readonly property color textSecondary: Qt.alpha(themeFg, 0.68)
    readonly property color textTertiary: Qt.alpha(themeFg, 0.45)

    readonly property int pageMargin: 40
    readonly property int cardGap: 14
    readonly property int sectionGap: 48

    // Fonte do tema (alias fontconfig: acompanha `omarchy font set`)
    readonly property string fontFamily: "monospace"

    readonly property int heroSize: 68
    readonly property int pageTitleSize: 28
    readonly property int sectionSize: 20
    readonly property int cardTitleSize: 14
    readonly property int metaSize: 13
    readonly property int bodySize: 15

    readonly property int radiusSm: omarchy.cornerRadius
    readonly property int radiusMd: omarchy.cornerRadius + 6
    readonly property int radiusLg: omarchy.cornerRadius + 10
    readonly property int radiusPill: 999

    readonly property int navRailWidth: 64
    readonly property int posterWidth: 176
    readonly property int landscapeWidth: 300
    readonly property int buttonHeight: 44
}
