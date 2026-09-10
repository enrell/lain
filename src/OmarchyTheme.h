#pragma once
#include <QColor>
#include <QFileSystemWatcher>
#include <QObject>

// Lê a paleta do Omarchy como o shell faz (theme/colors.toml) e expõe
// para o QML. Sem dependência de Quickshell: FileView/Process viram
// QFileSystemWatcher/QProcess. Recarrega ao trocar de tema.
class OmarchyTheme : public QObject {
    Q_OBJECT
    Q_PROPERTY(QColor foreground READ foreground NOTIFY themeChanged)
    Q_PROPERTY(QColor background READ background NOTIFY themeChanged)
    Q_PROPERTY(QColor accent READ accent NOTIFY themeChanged)
    Q_PROPERTY(QColor urgent READ urgent NOTIFY themeChanged)
    Q_PROPERTY(QColor muted READ muted NOTIFY themeChanged)
    Q_PROPERTY(bool dark READ dark NOTIFY themeChanged)
    Q_PROPERTY(int cornerRadius READ cornerRadius NOTIFY themeChanged)
public:
    explicit OmarchyTheme(QObject *parent = nullptr);

    QColor foreground() const { return m_foreground; }
    QColor background() const { return m_background; }
    QColor accent() const { return m_accent; }
    QColor urgent() const { return m_urgent; }
    QColor muted() const { return m_muted; }
    bool dark() const { return m_dark; }
    int cornerRadius() const { return m_cornerRadius; }

signals:
    void themeChanged();

private slots:
    void reload();
    void onRounded(int exitCode);

private:
    void load();

    QString m_colorsPath;
    QColor m_foreground = QColor("#F2F4F7");
    QColor m_background = QColor("#0A0C10");
    QColor m_accent = QColor("#89B4FA");
    QColor m_urgent = QColor("#F38BA8");
    QColor m_muted = QColor("#6B7480");
    bool m_dark = true;
    int m_cornerRadius = 8;
    QFileSystemWatcher m_watcher;
};
