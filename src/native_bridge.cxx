#include <cstdint>
#include <cstring>

#include <glib-object.h>

#include "ILoader.h"
#include "Scintilla.h"

struct RstpdNotification {
    uint32_t code;
    intptr_t position;
    int32_t ch;
    int32_t modification;
    int32_t updated;
    int32_t x;
    int32_t y;
    const char *text;
    size_t text_length;
};

extern "C" void rstpd_copy_notification(
    const SCNotification *source, RstpdNotification *destination, size_t text_limit) noexcept {
    *destination = {
        source->nmhdr.code,
        source->position,
        source->ch,
        source->modificationType,
        source->updated,
        source->x,
        source->y,
        nullptr,
        0,
    };
    if (source->nmhdr.code == SCN_URIDROPPED && source->text) {
        // ReceivedDrop appends a NUL; SCN_URIDROPPED does not populate length.
        destination->text = source->text;
        destination->text_length = strnlen(source->text, text_limit);
    }
}

extern "C" void rstpd_document_release(void *document) noexcept {
    static_cast<Scintilla::IDocumentEditable *>(document)->Release();
}

// The integration fixture uses the public signal and pinned ABI without a desktop clipboard.
extern "C" void rstpd_test_emit_uri_notification(void *widget, const char *text) noexcept {
    SCNotification notification {};
    notification.nmhdr.code = SCN_URIDROPPED;
    notification.text = text;
    g_signal_emit_by_name(widget, "sci-notify", 0, &notification);
}
