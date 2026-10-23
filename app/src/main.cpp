#include <QGuiApplication>
#include <QQmlContext>
#include <QQuickView>
#include <QScopedPointer>
#include <QtQml>

#include <sailfishapp.h>

#include "appservice.h"
#include "chatmodel.h"
#include "contactnames.h"
#include "daemon.h"
#include "messagemodel.h"

int main(int argc, char *argv[])
{
    QScopedPointer<QGuiApplication> app(SailfishApp::application(argc, argv));
    qmlRegisterUncreatableType<Daemon>("harbour.sailbubbles", 1, 0, "Daemon", "use the daemon context property");
    qmlRegisterType<MessageModel>("harbour.sailbubbles", 1, 0, "MessageModel");

    Daemon daemon;
    ChatModel chats(&daemon);
    ChatModel archived(&daemon, ChatModel::Archived);
    ChatModel unknown(&daemon, ChatModel::Unknown);
    ContactNames names(&daemon);
    AppService service;
    service.registerOnBus();

    QScopedPointer<QQuickView> view(SailfishApp::createView());
    view->rootContext()->setContextProperty("daemon", &daemon);
    view->rootContext()->setContextProperty("chatModel", &chats);
    view->rootContext()->setContextProperty("archivedChats", &archived);
    view->rootContext()->setContextProperty("unknownChats", &unknown);
    view->rootContext()->setContextProperty("appService", &service);
    view->rootContext()->setContextProperty("contacts", &names);
    view->setSource(SailfishApp::pathToMainQml());
    view->show();
    return app->exec();
}
