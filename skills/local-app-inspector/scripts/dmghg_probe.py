"""Inspect the user's installed DMGHG bridge without launching any desktop UI.

The bridge is loaded only from --app-dir. Its state is isolated in a new temporary
directory; no account/session database or persistent model settings are read.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit, urljoin
from urllib.request import Request, urlopen


class Bridge:
    READ_COMMANDS = {
        "app.ping", "catalog.get_channels", "catalog.get_banners",
        "catalog.get_video_list", "catalog.search_video", "catalog.get_video_key",
        "catalog.get_video_detail", "catalog.get_channel_detail", "video.play",
    }

    def __init__(self, app_dir):
        if os.name != "nt":
            raise RuntimeError("The installed bridge requires Windows x64 Python")
        self.app_dir = Path(app_dir).resolve(strict=True)
        self.dll_path = self.app_dir / "electron_bridge.dll"
        if not self.dll_path.is_file():
            raise RuntimeError("electron_bridge.dll is missing from the selected application")
        self.state_root = Path(tempfile.mkdtemp(prefix="dmghg-headless-"))
        os.environ["DMGHG_TEST_DATA_ROOT"] = str(self.state_root)
        os.environ["RUST_LOG"] = "error"
        self.dll = ctypes.CDLL(str(self.dll_path))
        self.dll.dmghg_service_new.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
        self.dll.dmghg_service_new.restype = ctypes.c_void_p
        self.dll.dmghg_service_handle_command_json.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        self.dll.dmghg_service_handle_command_json.restype = ctypes.c_void_p
        self.dll.dmghg_string_free.argtypes = [ctypes.c_void_p]
        self.dll.dmghg_service_free.argtypes = [ctypes.c_void_p]
        self.lock = threading.RLock()
        self.version = installed_version(self.app_dir / "dmghg.exe")
        self.handle = self.dll.dmghg_service_new(b"dmghg-flutter", self.version.encode("ascii"))
        if not self.handle:
            raise RuntimeError("The installed bridge could not initialize")

    def call(self, command, payload=None):
        if command not in self.READ_COMMANDS:
            raise ValueError("Command is outside the inspection allowlist")
        request = {"request_id": "headless-inspection", "protocol_version": 1,
                   "command": command, "payload": payload or {}}
        with self.lock:
            previous = Path.cwd()
            try:
                # The installed Lua resolver loads resources/lua relative to Cwd.
                os.chdir(self.app_dir)
                ptr = self.dll.dmghg_service_handle_command_json(
                    self.handle, json.dumps(request, ensure_ascii=False).encode("utf-8"))
                if not ptr:
                    raise RuntimeError("Bridge returned a null response")
                try:
                    response = json.loads(ctypes.string_at(ptr).decode("utf-8"))
                finally:
                    self.dll.dmghg_string_free(ptr)
            finally:
                os.chdir(previous)
        if response.get("ok") is not True:
            error = response.get("error", {})
            raise RuntimeError(f"{error.get('code')}: {error.get('message')}")
        return response.get("data")

    def close(self):
        if self.handle:
            self.dll.dmghg_service_free(self.handle)
            self.handle = None


def installed_version(exe):
    """Read PE version metadata; the resolver rejects unsupported client versions."""
    version = ctypes.windll.version
    size = version.GetFileVersionInfoSizeW(str(exe), None)
    if not size:
        raise RuntimeError("Cannot read the installed application's version metadata")
    buffer = ctypes.create_string_buffer(size)
    if not version.GetFileVersionInfoW(str(exe), 0, size, buffer):
        raise RuntimeError("Cannot read application version info")
    info = ctypes.c_void_p()
    length = ctypes.c_uint()
    if not version.VerQueryValueW(buffer, "\\", ctypes.byref(info), ctypes.byref(length)):
        raise RuntimeError("Application version structure is missing")
    fields = ctypes.cast(info, ctypes.POINTER(ctypes.c_uint32))
    if fields[0] != 0xFEEF04BD or length.value < 52:
        raise RuntimeError("Application version structure is invalid")
    return f"{fields[2] >> 16}.{fields[2] & 65535}.{fields[3] >> 16}"


def inspect(bridge, keyword):
    channels = bridge.call("catalog.get_channels")
    listing = bridge.call("catalog.get_video_list", {"page": 1})
    search = bridge.call("catalog.search_video", {"key": keyword, "page": "1"})
    videos = listing.get("items", [])
    if not channels or not videos or not search.get("items"):
        raise RuntimeError("Empty channel/list/search response; cannot certify a rule")
    selected = search["items"][0]
    detail = bridge.call("catalog.get_video_detail", {"id": str(selected["id"])})
    groups = detail.get("parts", [])
    episodes = sum(len(group.get("part", [])) for group in groups)
    if not episodes:
        raise RuntimeError("No real episodes in the detail response")
    group = next(g for g in groups if g.get("part"))
    payload = {"id": str(selected["id"]), "part": group["part"][0], "play": group["play"], "resolve": True}
    play = bridge.call("video.play", payload)
    variant = play["list"][play["default"]]
    media = media_probe(variant["url"], variant.get("headers"))
    return {
        "application": str(bridge.app_dir),
        "dll_sha256": hashlib.sha256(bridge.dll_path.read_bytes()).hexdigest(),
        "no_desktop": True, "installed_version": bridge.version,
        "isolated_state_root": str(bridge.state_root),
        "channels": len(channels), "list_items": len(videos),
        "search_keyword": keyword, "search_items": len(search["items"]),
        "detail_id": selected["id"], "detail_title": detail.get("name"),
        "episode_count": episodes, "play_variants": len(play["list"]),
        "media": media, "mobile_standalone_verified": False,
        "dependency": "Windows installed electron_bridge.dll and resources/lua",
    }, play


def media_probe(url, headers=None, depth=0):
    """Check actual bytes, never equate a play URL string with successful media."""
    request = Request(url, headers={**(headers or {}), "Range": "bytes=0-2047"})
    with urlopen(request, timeout=20) as response:
        chunk = response.read(2048)
        report = {"host": urlsplit(response.url).hostname, "status": response.status,
                "content_type": response.headers.get("Content-Type"), "bytes_read": len(chunk),
                "hls_manifest": chunk.lstrip().startswith(b"#EXTM3U")}
        if report["hls_manifest"] and depth < 2:
            text = chunk.decode("utf-8", "replace")
            uris = [line.strip() for line in text.splitlines() if line.strip() and not line.startswith("#")]
            if uris:
                report["first_child"] = media_probe(urljoin(response.url, uris[0]), headers, depth + 1)
        return report


def decode_probe(bridge, variant):
    """Use the installed libmpv decoder with null audio/video outputs; no window."""
    dll_dir = os.add_dll_directory(str(bridge.app_dir))
    mpv = ctypes.CDLL(str(bridge.app_dir / "libmpv-2.dll"))
    mpv.mpv_create.restype = ctypes.c_void_p
    mpv.mpv_set_option_string.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    mpv.mpv_initialize.argtypes = [ctypes.c_void_p]
    mpv.mpv_command.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_char_p)]
    mpv.mpv_get_property_string.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    mpv.mpv_get_property_string.restype = ctypes.c_void_p
    mpv.mpv_free.argtypes = [ctypes.c_void_p]
    mpv.mpv_wait_event.argtypes = [ctypes.c_void_p, ctypes.c_double]
    mpv.mpv_wait_event.restype = ctypes.c_void_p
    mpv.mpv_terminate_destroy.argtypes = [ctypes.c_void_p]
    handle = mpv.mpv_create()
    if not handle:
        raise RuntimeError("The installed media decoder could not initialize")
    try:
        for key, value in {"vo": "null", "ao": "null", "terminal": "no", "load-scripts": "no",
                           "network-timeout": "12", "cache-secs": "2", "demuxer-max-bytes": "4M"}.items():
            if mpv.mpv_set_option_string(handle, key.encode(), value.encode()) < 0:
                raise RuntimeError(f"Cannot set headless decoder option {key}")
        headers = ",".join(f"{k}: {v}" for k, v in (variant.get("headers") or {}).items())
        if headers:
            mpv.mpv_set_option_string(handle, b"http-header-fields", headers.encode())
        if mpv.mpv_initialize(handle) < 0:
            raise RuntimeError("The installed decoder initialization failed")
        args = (ctypes.c_char_p * 4)(b"loadfile", variant["url"].encode(), b"replace", None)
        if mpv.mpv_command(handle, args) < 0:
            raise RuntimeError("The installed decoder rejected the media request")
        deadline = time.monotonic() + 16
        facts = {}
        while time.monotonic() < deadline:
            mpv.mpv_wait_event(handle, 0.1)
            for key in ["audio-codec-name", "video-params/w", "video-params/h", "time-pos"]:
                ptr = mpv.mpv_get_property_string(handle, key.encode())
                if ptr:
                    try:
                        facts[key] = ctypes.string_at(ptr).decode("utf-8", "replace")
                    finally:
                        mpv.mpv_free(ptr)
            if facts.get("video-params/w") and float(facts.get("time-pos", "0")) > 0.2:
                break
        facts["decoded_video"] = bool(facts.get("video-params/w")) and float(facts.get("time-pos", "0")) > 0.2
        facts["outputs"] = {"audio": "null", "video": "null"}
        return facts
    finally:
        mpv.mpv_terminate_destroy(handle)
        dll_dir.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app-dir", required=True)
    parser.add_argument("--keyword", default="仙逆")
    parser.add_argument("--output", required=True)
    parser.add_argument("--show-play-schema", action="store_true")
    parser.add_argument("--decode", action="store_true", help="Verify actual video decoding with no window/audio output")
    args = parser.parse_args()
    bridge = Bridge(args.app_dir)
    try:
        report, play = inspect(bridge, args.keyword)
        if args.decode:
            report["decoder"] = decode_probe(bridge, play["list"][play["default"]])
        if args.show_play_schema:
            # Show keys/types and bounded non-URL metadata only; never dump signed links.
            print(json.dumps({k: type(v).__name__ for k, v in play.items()}, ensure_ascii=False))
        output = Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False))
        if args.decode and not report["decoder"]["decoded_video"]:
            raise RuntimeError("Media bytes exist but headless decoding was not verified")
    finally:
        bridge.close()


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
