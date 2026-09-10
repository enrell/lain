#include "OmarchyTheme.h"
#include <QDir>
#include <QFile>
#include <QProcess>
#include <QRegularExpression>
#include <QStandardPaths>

OmarchyTheme::OmarchyTheme(QObject *parent) : QObject(parent) {
    const QString home = QStandardPaths::writableLocation(QStandardPaths::HomeLocation);
    m_colorsPath = home + "/.local/state/omarchy/current/theme/colors.toml";
    load();

    // Tema ativo pode ser reescrito ou trocado; observa arquivo + diretório.
    m_watcher.addPath(m_colorsPath);
    m_watcher.addPath(QFileInfo(m_colorsPath).absolutePath());
    connect(&m_watcher, &QFileSystemWatcher::fileChanged, this, &OmarchyTheme::reload);
    connect(&m_watcher, &QFileSystemWatcher::directoryChanged, this, &OmarchyTheme::reload);

    // Espelha Hyprland decoration:rounding como o Style do shell faz.
    auto *proc = new QProcess(this);
    connect(proc, qOverload<int, QProcess::ExitStatus>(&QProcess::finished), this, &OmarchyTheme::onRounded);
    proc->start("hyprctl", {"-j", "getoption", "decoration:rounding"});
}

void OmarchyTheme::reload() {
    load();
    if (!m_watcher.files().contains(m_colorsPath) && QFile::exists(m_colorsPath))
        m_watcher.addPath(m_colorsPath);
    emit themeChanged();
}

void OmarchyTheme::onRounded(int exitCode) {
    auto *proc = qobject_cast<QProcess *>(sender());
    if (!proc || exitCode != 0) {
        proc->deleteLater();
        return;
    }
    static const QRegularExpression re("\"int\"\\s*:\\s*(\\d+)");
    auto m = re.match(QString::fromUtf8(proc->readAllStandardOutput()));
    if (m.hasMatch()) {
        m_cornerRadius = m.captured(1).toInt();
        emit themeChanged();
    }
    proc->deleteLater();
}

void OmarchyTheme::load() {
    QFile f(m_colorsPath);
    if (!f.open(QIODevice::ReadOnly | QIODevice::Text))
        return; // Fora do Omarchy: mantém defaults.

    // Mesma precedência do shell: chaves explícitas, depois color0/4/7/8.
    static const QRegularExpression kv("^\\s*([A-Za-z0-9_-]+)\\s*=\\s*[\"']?(#[0-9A-Fa-f]{6})");
    static const QRegularExpression mode("^\\s*mode\\s*=\\s*[\"']?(\\w+)");
    QString color0, color4, color7, color8;
    bool hasFg = false, hasBg = false, hasAccent = false, hasMuted = false;

    const QStringList lines = QString::fromUtf8(f.readAll()).split('\n');
    for (const QString &line : lines) {
        auto mm = mode.match(line);
        if (mm.hasMatch()) {
            m_dark = mm.captured(1).compare("light", Qt::CaseInsensitive) != 0;
            continue;
        }
        auto m = kv.match(line);
        if (!m.hasMatch())
            continue;
        const QString k = m.captured(1), v = m.captured(2);
        if (k == "foreground") { m_foreground = v; hasFg = true; }
        else if (k == "background") { m_background = v; hasBg = true; }
        else if (k == "accent") { m_accent = v; hasAccent = true; }
        else if (k == "muted") { m_muted = v; hasMuted = true; }
        else if (k == "red" || k == "color1") { m_urgent = v; }
        else if (k == "color0") color0 = v;
        else if (k == "color4") color4 = v;
        else if (k == "color7") color7 = v;
        else if (k == "color8") color8 = v;
    }
    if (!hasBg && !color0.isEmpty())
        m_background = QColor(color0);
    if (!hasFg && !color7.isEmpty())
        m_foreground = QColor(color7);
    if (!hasAccent && !color4.isEmpty())
        m_accent = QColor(color4);
    if (!hasMuted)
        m_muted = color8.isEmpty() ? m_foreground : QColor(color8);
}
