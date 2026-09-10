#include <QGuiApplication>
#include <QDebug>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <clocale>
#include "OmarchyTheme.h"
#include "ServerClient.h"

int main(int argc, char *argv[]) {
    std::setlocale(LC_NUMERIC, "C");
    QGuiApplication app(argc, argv);
    std::setlocale(LC_NUMERIC, "C"); // Qt pode restaurar o locale do ambiente
    qInfo() << "lain build" << LAIN_BUILD_TS;
    app.setApplicationName("lain");
    app.setOrganizationName("lain");

    ServerClient server;
    OmarchyTheme theme;
    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty("server", &server);
    engine.rootContext()->setContextProperty("omarchy", &theme);
    engine.rootContext()->setContextProperty("buildTs", QStringLiteral(LAIN_BUILD_TS));
    engine.loadFromModule("Lain", "Main");
    if (engine.rootObjects().isEmpty())
        return -1;
    return app.exec();
}
