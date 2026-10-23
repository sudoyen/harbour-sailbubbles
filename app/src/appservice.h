#ifndef APPSERVICE_H
#define APPSERVICE_H

#include <QObject>

// Called by notification taps (via lipstick) to open a chat. Sailjail lets
// the app own OrganizationName.ApplicationName on the session bus.
class AppService : public QObject
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.sailbubbles.App")

public:
    explicit AppService(QObject *parent = 0);
    bool registerOnBus();

    // Copies a received photo or video into Pictures or Videos/SailBubbles;
    // returns the new path, or an empty string.
    Q_INVOKABLE QString saveToGallery(const QString &path, const QString &mime);

public slots:
    Q_SCRIPTABLE void openChat(const QString &chatId);

signals:
    void openChatRequested(const QString &chatId);
};

#endif
