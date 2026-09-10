#pragma once
#include <QQuickFramebufferObject>
#include <QtQml/qqmlregistration.h>

#include <atomic>
#include <thread>

extern "C" {
#include <mpv/client.h>
#include <mpv/render_gl.h>
}

// Superfície de vídeo libmpv (Render API/OpenGL) + controle de playback.
// Uma thread consome mpv_wait_event e publica estado via queued invokes.
class MpvItem : public QQuickFramebufferObject {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(double position READ position NOTIFY positionChanged)
    Q_PROPERTY(double duration READ duration NOTIFY durationChanged)
    Q_PROPERTY(bool paused READ paused NOTIFY pausedChanged)
    Q_PROPERTY(double volume READ volume WRITE setVolume NOTIFY volumeChanged)
    Q_PROPERTY(bool muted READ muted WRITE setMuted NOTIFY mutedChanged)
    Q_PROPERTY(QVariantList audioTracks READ audioTracks NOTIFY tracksChanged)
    Q_PROPERTY(QVariantList subtitleTracks READ subtitleTracks NOTIFY tracksChanged)
    Q_PROPERTY(int audioId READ audioId NOTIFY tracksChanged)
    Q_PROPERTY(int subtitleId READ subtitleId NOTIFY tracksChanged)
    Q_PROPERTY(QString shaderPreset READ shaderPreset NOTIFY shaderChanged)
    Q_PROPERTY(QString shaderInfo READ shaderInfo NOTIFY shaderChanged)
    Q_PROPERTY(QStringList shaderModes READ shaderModes CONSTANT)
    Q_PROPERTY(QString errorText READ errorText NOTIFY errorTextChanged)
public:
    explicit MpvItem(QQuickItem *parent = nullptr);
    ~MpvItem() override;
    Renderer *createRenderer() const override;

    double position() const { return m_position; }
    double duration() const { return m_duration; }
    bool paused() const { return m_paused; }
    double volume() const { return m_volume; }
    bool muted() const { return m_muted; }
    QVariantList audioTracks() const { return m_audioTracks; }
    QVariantList subtitleTracks() const { return m_subtitleTracks; }
    int audioId() const { return m_audioId; }
    int subtitleId() const { return m_subtitleId; }
    QString shaderPreset() const { return m_shaderPreset; }
    QString shaderInfo() const { return m_shaderInfo; }
    QStringList shaderModes() const { return {"off", "fast", "balanced", "quality"}; }
    QString errorText() const { return m_errorText; }

    Q_INVOKABLE void play(const QString &url);
    Q_INVOKABLE void playTestPattern();
    Q_INVOKABLE void togglePause();
    Q_INVOKABLE void setPaused(bool p);
    Q_INVOKABLE void seek(double seconds);
    Q_INVOKABLE void seekBy(double delta);
    Q_INVOKABLE void setVolume(double v);
    Q_INVOKABLE void setMuted(bool m);
    Q_INVOKABLE void setAudioTrack(int id);
    Q_INVOKABLE void setSubtitleTrack(int id); // <0 = off
    Q_INVOKABLE void cycleAudio();
    Q_INVOKABLE void cycleSubtitle();
    Q_INVOKABLE void setShaderPreset(const QString &name);
    Q_INVOKABLE void cycleShaderPreset();

    friend class MpvRenderer;
signals:
    void positionChanged();
    void durationChanged();
    void pausedChanged();
    void volumeChanged();
    void mutedChanged();
    void tracksChanged();
    void shaderChanged();
    void errorTextChanged();

private:
    void eventLoop();
    void handleEvent(mpv_event *event);
    void runCmd(const QStringList &args);
    void setProp(const QString &name, const QString &value);
    void applyShaderPreset(const QString &name);
    static QVariant nodeToVariant(const mpv_node *node);
    static QString findShaderDir();

    mpv_handle *m_mpv = nullptr;
    mpv_render_context *m_ctx = nullptr;
    std::thread m_thread;
    std::atomic<bool> m_running{false};

    double m_position = 0.0;
    double m_duration = 0.0;
    bool m_paused = false;
    double m_volume = 100.0;
    bool m_muted = false;
    QVariantList m_audioTracks;
    QVariantList m_subtitleTracks;
    int m_audioId = -1;
    int m_subtitleId = -1;
    QString m_shaderPreset = "off";
    QString m_shaderInfo = "Anime4K off";
    QString m_errorText;
};
