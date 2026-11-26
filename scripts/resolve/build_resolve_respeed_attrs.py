#!/usr/bin/env python3
"""
Generates a DaVinci Resolve 21.2 reference corpus for respeed & Clip Attributes:
  - 50% slow-mo (+ PitchCorrection=True + Optical Flow / Speed Warp Better)
  - 200% fast-forward (+ PitchCorrection=False + Frame Blend + Scale Fill)
  - -100% reverse playback
  - 0% freeze frame
  - Clip Attributes: FPS interpret override, Start TC override, Data Level=Full,
    Alpha mode=Straight, and SetAudioMapping channel mute
  - TimelineItem Inspector attributes: FlipX/FlipY, CompositeMode=Screen, Audio Volume/Pan/Pitch
"""

import argparse
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, "/Library/Application Support/Blackmagic Design/DaVinci Resolve/Developer/Scripting/Modules")
import DaVinciResolveScript as dvr

PROJECT_NAME = "NLE_Converter_Respeed_Attrs_21"


def main():
    parser = argparse.ArgumentParser(description="Generate Resolve 21.2 respeed & clip attributes corpus")
    parser.add_argument("--media-dir", type=Path, required=True, help="Directory containing test footage")
    parser.add_argument("--output-dir", type=Path, required=True, help="Output directory for exported files")
    args = parser.parse_args()

    media_dir: Path = args.media_dir
    output_dir: Path = args.output_dir
    output_dir.mkdir(parents=True, exist_ok=True)

    resolve = dvr.scriptapp("Resolve")
    if resolve is None:
        raise RuntimeError("DaVinci Resolve must be running")

    pm = resolve.GetProjectManager()
    orig = pm.GetCurrentProject()
    orig_name = orig.GetName() if orig else None

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

        mp = project.GetMediaPool()
        root = mp.GetRootFolder()
        base_bin = mp.AddSubFolder(root, "01_Baseline_Unmodified")
        attr_bin = mp.AddSubFolder(root, "02_Modified_ClipAttributes")
        tl_bin = mp.AddSubFolder(root, "03_Timelines")

        mp.SetCurrentFolder(base_bin)
        base_clips = mp.ImportMedia([
            str(media_dir / "ShortPlantHDR.mp4"),
            str(media_dir / "TimeCode_HEVC.mov"),
            str(media_dir / "HDR_MultiAudioTrack.mov"),
            str(media_dir / "Plant_TV_ProRes_Alpha.mov"),
        ]) or []
        base_by_name = {c.GetName(): c for c in base_clips}

        mp.SetCurrentFolder(attr_bin)
        mod_candidates = [
            media_dir / "ShortPlantHDR_av1_h265_tv.mov",
            media_dir / "TimeCode_HEVC_h265_h265_tv.mov",
        ]
        mod_clips = mp.ImportMedia([str(p) for p in mod_candidates if p.exists()]) or []
        mod_by_name = {c.GetName(): c for c in mod_clips}

        plant_mod = mod_by_name.get("ShortPlantHDR_av1_h265_tv.mov") or base_by_name["ShortPlantHDR.mp4"]
        alpha_clip = base_by_name["Plant_TV_ProRes_Alpha.mov"]
        hdr_clip = base_by_name["HDR_MultiAudioTrack.mov"]
        tc_base = base_by_name["TimeCode_HEVC.mov"]

        plant_mod.SetClipProperty("FPS", "25.0")
        plant_mod.SetClipProperty("Data Level", "Full")
        plant_mod.SetClipProperty("Start TC", "10:00:00:00")
        alpha_clip.SetClipProperty("Alpha mode", "Straight")

        hdr_amap_raw = hdr_clip.GetAudioMapping()
        if hdr_amap_raw:
            amap = json.loads(hdr_amap_raw)
            if "track_mapping" in amap and isinstance(amap["track_mapping"], dict):
                keys = list(amap["track_mapping"].keys())
                if len(keys) >= 2:
                    amap["track_mapping"][keys[1]]["mute"] = True
                hdr_clip.SetAudioMapping(json.dumps(amap))

        mp.SetCurrentFolder(tl_bin)
        tl = mp.CreateEmptyTimeline("05_Respeed_And_ClipAttributes")
        project.SetCurrentTimeline(tl)
        tl.SetStartTimecode("01:00:00:00")
        tl.AddTrack("video")
        t0 = tl.GetStartFrame()

        mp.AppendToTimeline([{"mediaPoolItem": tc_base, "startFrame": 0, "endFrame": 99, "trackIndex": 1, "recordFrame": t0}])
        mp.AppendToTimeline([{"mediaPoolItem": tc_base, "startFrame": 100, "endFrame": 199, "trackIndex": 1, "recordFrame": t0 + 100}])
        mp.AppendToTimeline([{"mediaPoolItem": tc_base, "startFrame": 200, "endFrame": 299, "trackIndex": 1, "recordFrame": t0 + 200}])
        mp.AppendToTimeline([{"mediaPoolItem": tc_base, "startFrame": 300, "endFrame": 399, "trackIndex": 1, "recordFrame": t0 + 300}])
        mp.AppendToTimeline([{"mediaPoolItem": tc_base, "startFrame": 400, "endFrame": 474, "trackIndex": 1, "recordFrame": t0 + 400}])
        mp.AppendToTimeline([{"mediaPoolItem": plant_mod, "startFrame": 0, "endFrame": 99, "trackIndex": 1, "recordFrame": t0 + 475}])
        mp.AppendToTimeline([{"mediaPoolItem": hdr_clip, "startFrame": 0, "endFrame": 99, "trackIndex": 1, "recordFrame": t0 + 575}])
        mp.AppendToTimeline([{"mediaPoolItem": alpha_clip, "startFrame": 0, "endFrame": 74, "mediaType": 1, "trackIndex": 2, "recordFrame": t0 + 475}])

        v1 = tl.GetItemListInTrack("video", 1) or []
        v2 = tl.GetItemListInTrack("video", 2) or []
        a1 = tl.GetItemListInTrack("audio", 1) or []

        if len(v1) >= 6:
            v1[1].SetSpeed({"Percentage": 50.0, "PitchCorrection": True, "RippleTimeline": False})
            v1[1].SetProperties({
                "RetimeAndScalingEnabled": True,
                "RetimeProcess": resolve.RETIME_OPTICAL_FLOW,
                "MotionEstimation": resolve.MOTION_EST_SPEED_WARP_BETTER,
            })
            v1[2].SetSpeed({"Percentage": 200.0, "PitchCorrection": False, "RippleTimeline": False})
            v1[2].SetProperties({
                "RetimeAndScalingEnabled": True,
                "RetimeProcess": resolve.RETIME_FRAME_BLEND,
                "Scaling": resolve.SCALE_FILL,
            })
            v1[3].SetSpeed({"Percentage": -100.0, "PitchCorrection": False, "RippleTimeline": False})
            v1[4].SetSpeed({"Percentage": 0.0, "RippleTimeline": False})
            v1[5].SetProperties({"TransformEnabled": True, "FlipX": True, "RetimeAndScalingEnabled": True, "Scaling": resolve.SCALE_FIT})

        if v2:
            v2[0].SetProperties({
                "TransformEnabled": True,
                "FlipY": True,
                "CompositeEnabled": True,
                "CompositeMode": resolve.COMPOSITE_SCREEN,
                "Opacity": 75.0,
            })

        if a1:
            a1[0].SetProperties({
                "AudioVolumeEnabled": True,
                "AudioVolume": -6.5,
                "AudioPanEnabled": True,
                "AudioPan": -35.0,
                "AudioPitchEnabled": True,
                "AudioPitchSemiTones": 2.0,
            })

        for fn, et, es in [
            ("05_Respeed_And_ClipAttributes.fcpxml", resolve.EXPORT_FCPXML_1_10, resolve.EXPORT_NONE),
            ("05_Respeed_And_ClipAttributes_fcp7.xml", resolve.EXPORT_FCP_7_XML, resolve.EXPORT_NONE),
            ("05_Respeed_And_ClipAttributes.otio", resolve.EXPORT_OTIO, resolve.EXPORT_NONE),
            ("05_Respeed_And_ClipAttributes.drt", resolve.EXPORT_DRT, resolve.EXPORT_NONE),
        ]:
            target = str(output_dir / fn)
            ok = tl.Export(target, et, es)
            print(f"Export {fn}: ok={ok} exists={os.path.exists(target)}")

        pm.SaveProject()
        drp_path = str(output_dir / "Resolve21_Respeed_Attrs.drp")
        ok_drp = pm.ExportProject(PROJECT_NAME, drp_path, True)
        print(f"ExportProject {drp_path}: ok={ok_drp}")
    finally:
        if orig_name and orig_name != PROJECT_NAME:
            pm.LoadProject(orig_name)
        else:
            pm.CloseProject(project)
        pm.DeleteProject(PROJECT_NAME)


if __name__ == "__main__":
    main()
