#pragma once
#include <QObject>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

// Contrato mínimo que o backend vai precisar cumprir (Phase 1: Home).
// Hoje retorna MOCK local; quando o servidor existir, troca-se a
// implementação sem tocar no QML.
//
// Endpoints futuros espelhando isto:
//   GET /api/v1/home             -> hero + rows
//   GET /api/v1/media/:id        -> detalhe
//   GET /api/v1/stream/:id       -> url p/ libmpv
class ServerClient : public QObject {
    Q_OBJECT
public:
    explicit ServerClient(QObject *parent = nullptr);

    Q_INVOKABLE QVariantMap home();
    Q_INVOKABLE QVariantMap media(const QString &id);
    Q_INVOKABLE QVariantList movies();
    Q_INVOKABLE QString streamUrl(const QString &id);
};
