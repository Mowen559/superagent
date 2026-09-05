#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct SnowCaptureDesktopSessionImpl SnowCaptureDesktopSession;
typedef struct SnowCaptureRegionSessionImpl SnowCaptureRegionSession;
typedef struct SnowCaptureSnapshotImpl SnowCaptureSnapshot;
typedef struct SnowCaptureFrameLeaseImpl SnowCaptureFrameLease;
typedef struct SnowCaptureRecordingSessionImpl SnowCaptureRecordingSession;

typedef struct SnowCaptureDesktopSessionConfig {
    size_t capture_retry_count;
    uint8_t reserved[32];
} SnowCaptureDesktopSessionConfig;

typedef struct SnowCaptureDesktopSessionState {
    size_t worker_count;
    uint8_t prepared;
    uint8_t reserved0[7];
    uint64_t retained_resource_bytes;
    const char* backend_kind;
} SnowCaptureDesktopSessionState;

typedef struct SnowCaptureFrameInfo {
    const char* stable_id;
    const char* name;
    int32_t x;
    int32_t y;
    uint32_t width;
    uint32_t height;
    uint8_t is_primary;
    uint8_t reserved0[3];
    uint32_t stride_bytes;
    const uint8_t* rgba_bytes;
    size_t rgba_len;
} SnowCaptureFrameInfo;

typedef struct SnowCaptureRegionSessionConfig {
    int32_t x;
    int32_t y;
    uint32_t width;
    uint32_t height;
    size_t capture_retry_count;
    uint8_t reserved[32];
} SnowCaptureRegionSessionConfig;

typedef struct SnowCaptureRegionFrameInfo {
    uint32_t width;
    uint32_t height;
    uint32_t stride_bytes;
    uint8_t is_duplicate;
    uint8_t reserved0[3];
    const uint8_t* rgba_bytes;
    size_t rgba_len;
} SnowCaptureRegionFrameInfo;

typedef struct SnowCaptureRecordingConfig {
    int32_t x;
    int32_t y;
    uint32_t width;
    uint32_t height;
    uint32_t fps;
    uint8_t enable_microphone;
    uint8_t enable_system_audio;
    uint8_t reserved0[2];
    const char* working_directory_utf8;
    uint8_t reserved[32];
} SnowCaptureRecordingConfig;

typedef enum SnowCaptureRecordingState {
    SNOW_CAPTURE_RECORDING_STATE_CREATED = 0,
    SNOW_CAPTURE_RECORDING_STATE_RUNNING = 1,
    SNOW_CAPTURE_RECORDING_STATE_PAUSED = 2,
    SNOW_CAPTURE_RECORDING_STATE_STOPPED = 3,
} SnowCaptureRecordingState;

SnowCaptureDesktopSession* snow_capture_desktop_session_create(
    const SnowCaptureDesktopSessionConfig* config);
void snow_capture_desktop_session_destroy(SnowCaptureDesktopSession* session);

uint8_t snow_capture_desktop_session_prepare(SnowCaptureDesktopSession* session);
uint8_t snow_capture_desktop_session_state(
    SnowCaptureDesktopSession* session,
    SnowCaptureDesktopSessionState* out_state);
uint8_t snow_capture_desktop_session_refresh_layout(SnowCaptureDesktopSession* session);
uint8_t snow_capture_desktop_session_release_idle_resources(SnowCaptureDesktopSession* session);
SnowCaptureSnapshot* snow_capture_desktop_session_capture_all(
    SnowCaptureDesktopSession* session);

SnowCaptureRegionSession* snow_capture_region_session_create(
    const SnowCaptureRegionSessionConfig* config);
void snow_capture_region_session_destroy(SnowCaptureRegionSession* session);
uint8_t snow_capture_region_session_prepare(SnowCaptureRegionSession* session);
/* The returned pixel pointer remains valid until the next capture or destroy. */
uint8_t snow_capture_region_session_capture(
    SnowCaptureRegionSession* session,
    SnowCaptureRegionFrameInfo* out_info);

size_t snow_capture_snapshot_count(const SnowCaptureSnapshot* snapshot);
uint8_t snow_capture_snapshot_frame_info(
    const SnowCaptureSnapshot* snapshot,
    size_t index,
    SnowCaptureFrameInfo* out_info);
SnowCaptureFrameLease* snow_capture_snapshot_frame_retain(
    const SnowCaptureSnapshot* snapshot,
    size_t index);
void snow_capture_frame_lease_release(SnowCaptureFrameLease* lease);
void snow_capture_snapshot_destroy(SnowCaptureSnapshot* snapshot);

SnowCaptureRecordingSession* snow_capture_recording_session_create(
    const SnowCaptureRecordingConfig* config);
void snow_capture_recording_session_destroy(SnowCaptureRecordingSession* session);
uint8_t snow_capture_recording_session_start(SnowCaptureRecordingSession* session);
uint8_t snow_capture_recording_session_pause(SnowCaptureRecordingSession* session);
uint8_t snow_capture_recording_session_resume(SnowCaptureRecordingSession* session);
uint8_t snow_capture_recording_session_state(
    const SnowCaptureRecordingSession* session,
    SnowCaptureRecordingState* out_state);
uint8_t snow_capture_recording_session_stop_and_export(
    SnowCaptureRecordingSession* session,
    const char* output_file_utf8,
    uint8_t export_gif);

const char* snow_capture_last_error_message(void);

#ifdef __cplusplus
}
#endif
