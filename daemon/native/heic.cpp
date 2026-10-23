#include <libheif/heif.h>

#include <QtCore/QString>
#include <QtGui/QImage>

#include <cstring>

namespace {

int fail(const char *msg, char *err, size_t errlen)
{
    qstrncpy(err, msg, errlen);
    return -1;
}

}

// Decodes the primary image of the HEIC file `in` (rotation and mirroring
// applied, so it comes out upright) and writes it to `out` as JPEG.
// Returns 0 on success, -1 with err set otherwise.
extern "C" int obd_heic_to_jpeg(const char *in, const char *out, char *err, size_t errlen)
{
    heif_context *ctx = heif_context_alloc();
    heif_image_handle *handle = 0;
    heif_image *image = 0;
    int rc = -1;

    heif_error e = heif_context_read_from_file(ctx, in, 0);
    if (e.code == heif_error_Ok)
        e = heif_context_get_primary_image_handle(ctx, &handle);
    if (e.code == heif_error_Ok)
        e = heif_decode_image(handle, &image, heif_colorspace_RGB, heif_chroma_interleaved_RGB, 0);
    if (e.code != heif_error_Ok) {
        rc = fail(e.message, err, errlen);
    } else {
        int stride = 0;
        const uint8_t *data = heif_image_get_plane_readonly(image, heif_channel_interleaved, &stride);
        const int width = heif_image_get_width(image, heif_channel_interleaved);
        const int height = heif_image_get_height(image, heif_channel_interleaved);
        const QImage picture(data, width, height, stride, QImage::Format_RGB888);
        rc = picture.save(QString::fromUtf8(out), "JPEG", 90) ? 0 : fail("writing the JPEG failed", err, errlen);
    }

    if (image)
        heif_image_release(image);
    if (handle)
        heif_image_handle_release(handle);
    heif_context_free(ctx);
    return rc;
}
