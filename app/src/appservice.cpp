#include "appservice.h"

#include <QDBusConnection>
#include <QDBusError>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QStandardPaths>

AppService::AppService(QObject *parent)
    : QObject(parent)
{
}

bool AppService::registerOnBus()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.registerObject("/", this, QDBusConnection::ExportScriptableSlots)
            || !bus.registerService("org.sailbubbles.harbour-sailbubbles")) {
        qWarning() << "AppService registration failed:" << bus.lastError().name();
        return false;
    }
    return true;
}

QString AppService::saveToGallery(const QString &path, const QString &mime)
{
    const QStandardPaths::StandardLocation location = mime.startsWith("video/")
        ? QStandardPaths::MoviesLocation : QStandardPaths::PicturesLocation;
    const QDir dir(QStandardPaths::writableLocation(location) + "/SailBubbles");
    if (!dir.mkpath("."))
        return QString();
    const QFileInfo source(path);
    // the stored names start with the attachment's index ("0-IMG_1.jpg")
    const QString name = source.fileName().section('-', 1);
    QString target = dir.filePath(name);
    for (int n = 1; QFile::exists(target); ++n)
        target = dir.filePath(QString("%1-%2.%3").arg(source.completeBaseName().section('-', 1)).arg(n).arg(source.suffix()));
    return QFile::copy(path, target) ? target : QString();
}

void AppService::openChat(const QString &chatId)
{
    emit openChatRequested(chatId);
}
