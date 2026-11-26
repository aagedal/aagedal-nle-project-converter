#!/usr/bin/env python3
"""
Builds a comprehensive DaVinci Resolve 21.2 reference corpus using the official
DaVinciResolveScript Python API:
  - 01_MultiTrack_FX_Audio: V1/V2, stereo + mono audio, transitions, transforms, speed, fades, colors, markers
  - 02_Compound_Clips: native Sm2MpCompoundClip created with CreateCompoundClip
  - 03_Nested_Timeline: nested Sub_B_Roll timeline placed inside a parent timeline
  - 04_Multicam: 3-angle Sm2MpMulticamClip with live SmartSwitch angle cuts

Exports Resolve21_Corpus.drp plus .drt, .fcpxml (1.10 bundle), FCP7 _fcp7.xml, and .otio
for each timeline.
"""

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

RESOLVE_SCRIPT_MODULES = "/Library/Application Support/Blackmagic Design/DaVinci Resolve/Developer/Scripting/Modules"
sys.path.insert(0, RESOLVE_SCRIPT_MODULES)

PROJECT_NAME = "NLE_Converter_Corpus_21"


def connect_resolve(timeout_sec: int = 90):
    import DaVinciResolveScript as dvr

    resolve = dvr.scriptapp("Resolve")
    if resolve is not None:
        return resolve

    print("Launching DaVinci Resolve in background...")
    subprocess.run(
        ["open", "-g", "-a", "/Applications/DaVinci Resolve/DaVinci Resolve.app"],
        check=True,
    )
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        time.sleep(2.0)
        resolve = dvr.scriptapp("Resolve")
        if resolve is not None:
            time.sleep(2.0)
            return resolve
    raise RuntimeError(f"Timed out waiting {timeout_sec}s for DaVinci Resolve scripting API")


def export_timeline_formats(resolve, timeline, out_dir: Path, slug: str):
    exports = [
        (f"{slug}.fcpxml", resolve.EXPORT_FCPXML_1_10, resolve.EXPORT_NONE),
        (f"{slug}_fcp7.xml", resolve.EXPORT_FCP_7_XML, resolve.EXPORT_NONE),
        (f"{slug}.otio", resolve.EXPORT_OTIO, resolve.EXPORT_NONE),
        (f"{slug}.drt", resolve.EXPORT_DRT, resolve.EXPORT_NONE),
    ]
    for filename, exp_type, exp_sub in exports:
        target = str(out_dir / filename)
        ok = timeline.Export(target, exp_type, exp_sub)
        exists = os.path.exists(target)
        print(f"  Export {filename}: ok={ok} exists={exists}")


def main():
    parser = argparse.ArgumentParser(description="Generate DaVinci Resolve 21.2 reference corpus")
    parser.add_argument("--media-dir", type=Path, required=True, help="Directory containing test footage")
    parser.add_argument("--output-dir", type=Path, required=True, help="Output directory for exported files")
    args = parser.parse_args()

    media_dir: Path = args.media_dir
    output_dir: Path = args.output_dir
    output_dir.mkdir(parents=True, exist_ok=True)

    resolve = connect_resolve()
    print(f"Connected to DaVinci Resolve {resolve.GetVersionString()}")

    pm = resolve.GetProjectManager()
    orig_project = pm.GetCurrentProject()
    orig_project_name = orig_project.GetName() if orig_project else None

    if pm.LoadProject(PROJECT_NAME):
        pm.CloseProject(pm.GetCurrentProject())
        pm.DeleteProject(PROJECT_NAME)

    project = pm.CreateProject(PROJECT_NAME)
    if project is None:
        raise RuntimeError(f"Failed to create project {PROJECT_NAME}")

    try:
        project.SetSetting("timelineFrameRate", "25")
        project.SetSetting("timelineResolutionWidth", "1920")
        project.SetSetting("timelineResolutionHeight", "1080")
        project.SetSetting("timelinePlaybackFrameRate", "25")

        mp = project.GetMediaPool()
        root_folder = mp.GetRootFolder()

        footage_bin = mp.AddSubFolder(root_folder, "01_Footage")
        cam_bin = mp.AddSubFolder(footage_bin, "Cameras")
        audio_bin = mp.AddSubFolder(footage_bin, "Audio")
        tl_bin = mp.AddSubFolder(root_folder, "02_Timelines")
        cc_mc_bin = mp.AddSubFolder(root_folder, "03_Compounds_Multicam")

        mp.SetCurrentFolder(cam_bin)
        cam_files = [
            str(media_dir / "TimeCode_HEVC.mov"),
            str(media_dir / "ShortPlantHDR.mp4"),
            str(media_dir / "HDR_MultiAudioTrack.mov"),
        ]
        cam_clips = mp.ImportMedia(cam_files) or []
        if len(cam_clips) < 3:
            raise RuntimeError(f"Expected 3 camera clips in {media_dir}, got {len(cam_clips)}")

        clip_by_name = {c.GetName(): c for c in cam_clips}
        tc_clip = clip_by_name["TimeCode_HEVC.mov"]
        plant_clip = clip_by_name["ShortPlantHDR.mp4"]
        hdr_clip = clip_by_name["HDR_MultiAudioTrack.mov"]

        mp.SetCurrentFolder(audio_bin)
        audio_clips = mp.ImportMedia([str(media_dir / "TV 2 Nyhetsbed 1.mp3")]) or []
        music_clip = audio_clips[0]

        tc_clip.SetClipColor("Purple")
        plant_clip.SetClipColor("Lime")
        hdr_clip.SetClipColor("Orange")

        # 1. Multi-track + FX + stereo/mono audio
        mp.SetCurrentFolder(tl_bin)
        tl1 = mp.CreateEmptyTimeline("01_MultiTrack_FX_Audio")
        project.SetCurrentTimeline(tl1)
        tl1.SetStartTimecode("01:00:00:00")
        tl1.AddTrack("video")
        tl1.AddTrack("audio", "mono")
        tl1.AddTrack("audio", "stereo")
        tl1.SetTrackName("video", 1, "Main Video")
        tl1.SetTrackName("video", 2, "B-Roll Overlay")
        tl1.SetTrackName("audio", 1, "Dialogue Stereo")
        tl1.SetTrackName("audio", 2, "Mono Mic")
        tl1.SetTrackName("audio", 3, "Music Bed")

        t0 = tl1.GetStartFrame()
        mp.AppendToTimeline([{"mediaPoolItem": tc_clip, "startFrame": 25, "endFrame": 149, "trackIndex": 1, "recordFrame": t0}])
        mp.AppendToTimeline([{"mediaPoolItem": plant_clip, "startFrame": 50, "endFrame": 199, "trackIndex": 1, "recordFrame": t0 + 125}])
        mp.AppendToTimeline([{"mediaPoolItem": hdr_clip, "startFrame": 100, "endFrame": 224, "mediaType": 1, "trackIndex": 1, "recordFrame": t0 + 275}])
        mp.AppendToTimeline([{"mediaPoolItem": hdr_clip, "startFrame": 300, "endFrame": 424, "mediaType": 1, "trackIndex": 2, "recordFrame": t0 + 75}])
        mp.AppendToTimeline([{"mediaPoolItem": tc_clip, "startFrame": 100, "endFrame": 224, "mediaType": 2, "trackIndex": 2, "recordFrame": t0 + 50}])
        mp.AppendToTimeline([{"mediaPoolItem": music_clip, "startFrame": 0, "endFrame": 399, "mediaType": 2, "trackIndex": 3, "recordFrame": t0}])

        v1_items = tl1.GetItemListInTrack("video", 1) or []
        v2_items = tl1.GetItemListInTrack("video", 2) or []
        a1_items = tl1.GetItemListInTrack("audio", 1) or []
        a3_items = tl1.GetItemListInTrack("audio", 3) or []

        if len(v1_items) >= 2:
            v1_items[0].AddTransition({"type": "Cross Dissolve", "category": "simple", "position": "end", "alignment": "center", "duration": 25})
            v1_items[1].SetProperties({"ZoomX": 1.35, "ZoomY": 1.35, "Pan": 120.0, "Tilt": -60.0, "RotationAngle": 5.5, "CropLeft": 80.0})
            v1_items[1].SetClipColor("Teal")
            v1_items[1].AddMarker(30, "Green", "ClipNote", "Transformed 4K plant clip", 10)
        if len(v1_items) >= 3:
            v1_items[2].SetSpeed({"Percentage": 50.0, "RippleTimeline": False})
        if v2_items:
            v2_items[0].SetProperties({"ZoomX": 0.45, "ZoomY": 0.45, "Pan": 420.0, "Tilt": 240.0, "Opacity": 85.0})
            v2_items[0].SetFades({"FadeIn": 12, "FadeOut": 12})
        if len(a1_items) >= 2:
            a1_items[0].AddTransition({"type": "Cross Fade +3 dB", "category": "audio", "position": "end", "alignment": "center", "duration": 20})
        if a3_items:
            a3_items[0].SetFades({"FadeIn": 25, "FadeOut": 25})

        tl1.AddMarker(50, "Blue", "Intro Cut", "Timeline marker on V1/V2 overlap", 1)
        tl1.AddMarker(200, "Red", "Review Effect", "Check transform and speed", 15)
        export_timeline_formats(resolve, tl1, output_dir, "01_MultiTrack_FX_Audio")

        # 2. Compound clip
        mp.SetCurrentFolder(tl_bin)
        tl2 = mp.CreateEmptyTimeline("02_Compound_Clips")
        project.SetCurrentTimeline(tl2)
        tl2.SetStartTimecode("01:00:00:00")
        tl2.AddTrack("video")
        t2_0 = tl2.GetStartFrame()
        mp.AppendToTimeline([
            {"mediaPoolItem": tc_clip, "startFrame": 0, "endFrame": 99, "trackIndex": 1, "recordFrame": t2_0},
            {"mediaPoolItem": plant_clip, "startFrame": 0, "endFrame": 99, "trackIndex": 1, "recordFrame": t2_0 + 100},
            {"mediaPoolItem": hdr_clip, "startFrame": 50, "endFrame": 149, "mediaType": 1, "trackIndex": 2, "recordFrame": t2_0 + 50},
        ])
        cc_inputs = (
            (tl2.GetItemListInTrack("video", 1) or [])
            + (tl2.GetItemListInTrack("video", 2) or [])
            + (tl2.GetItemListInTrack("audio", 1) or [])
        )
        tl2.CreateCompoundClip(cc_inputs, {"name": "CC_Intro_Sequence", "startTimecode": "00:00:00:00"})
        tl2.AddMarker(100, "Blue", "Inside Compound Range", "Parent timeline marker", 1)
        export_timeline_formats(resolve, tl2, output_dir, "02_Compound_Clips")

        # 3. Nested timeline
        mp.SetCurrentFolder(cc_mc_bin)
        sub_tl = mp.CreateEmptyTimeline("Sub_B_Roll")
        project.SetCurrentTimeline(sub_tl)
        sub_tl.SetStartTimecode("00:00:00:00")
        sub_t0 = sub_tl.GetStartFrame()
        mp.AppendToTimeline([
            {"mediaPoolItem": plant_clip, "startFrame": 0, "endFrame": 74, "trackIndex": 1, "recordFrame": sub_t0},
            {"mediaPoolItem": hdr_clip, "startFrame": 0, "endFrame": 74, "trackIndex": 1, "recordFrame": sub_t0 + 75},
        ])
        sub_v1 = sub_tl.GetItemListInTrack("video", 1) or []
        if sub_v1:
            sub_v1[0].AddTransition({"type": "Cross Dissolve", "category": "simple", "position": "end", "alignment": "center", "duration": 15})
        sub_tl.AddMarker(35, "Purple", "SubMarker", "Inside nested timeline", 1)
        sub_mp_item = sub_tl.GetMediaPoolItem()

        mp.SetCurrentFolder(tl_bin)
        tl3 = mp.CreateEmptyTimeline("03_Nested_Timeline")
        project.SetCurrentTimeline(tl3)
        tl3.SetStartTimecode("01:00:00:00")
        t3_0 = tl3.GetStartFrame()
        mp.AppendToTimeline([{"mediaPoolItem": tc_clip, "startFrame": 0, "endFrame": 99, "trackIndex": 1, "recordFrame": t3_0}])
        if sub_mp_item is not None:
            mp.AppendToTimeline([{"mediaPoolItem": sub_mp_item, "startFrame": 10, "endFrame": 134, "trackIndex": 1, "recordFrame": t3_0 + 100}])
        export_timeline_formats(resolve, tl3, output_dir, "03_Nested_Timeline")

        # 4. Multicam clip + SmartSwitch
        mp.SetCurrentFolder(cc_mc_bin)
        mc_clips = mp.CreateMulticamClip(
            [tc_clip, plant_clip, hdr_clip],
            {
                "name": "MC_3Cam_Test",
                "startTimecode": "00:00:00:00",
                "frameRate": 25.0,
                "angleSyncMode": resolve.MULTICAM_ANGLE_SYNC_IN,
                "angleNameMode": resolve.MULTICAM_ANGLE_NAME_CLIP,
                "multicamAudioMode": resolve.MULTICAM_AUDIO_SOURCE,
                "createBinForSourceClips": False,
            },
        ) or []

        mp.SetCurrentFolder(tl_bin)
        tl4 = mp.CreateEmptyTimeline("04_Multicam")
        project.SetCurrentTimeline(tl4)
        tl4.SetStartTimecode("01:00:00:00")
        t4_0 = tl4.GetStartFrame()
        if mc_clips:
            mc_items = mp.AppendToTimeline([{"mediaPoolItem": mc_clips[0], "startFrame": 0, "endFrame": 374, "trackIndex": 1, "recordFrame": t4_0}]) or []
            if mc_items:
                mc_items[0].PerformMulticamSmartSwitch({
                    "analysisMode": resolve.SMART_SWITCH_ANALYSIS_MODE_AUDIO_ONLY,
                    "isAutoDetectWideAngle": False,
                    "wideAngleID": "None",
                })
        export_timeline_formats(resolve, tl4, output_dir, "04_Multicam")

        project.SetCurrentTimeline(tl1)
        pm.SaveProject()
        drp_path = str(output_dir / "Resolve21_Corpus.drp")
        ok_drp = pm.ExportProject(PROJECT_NAME, drp_path, True)
        print(f"ExportProject {drp_path}: ok={ok_drp}")
    finally:
        if orig_project_name and orig_project_name != PROJECT_NAME:
            pm.LoadProject(orig_project_name)
        else:
            pm.CloseProject(project)
        pm.DeleteProject(PROJECT_NAME)


if __name__ == "__main__":
    main()
