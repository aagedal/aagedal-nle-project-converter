#!/usr/bin/env python3
"""
Builds a comprehensive Adobe Premiere Pro 2026 reference corpus using a temporary
local CEP 12 ExtendScript runner (automatically cleaned up on exit):
  - 01_MultiTrack_FX_Motion_Lumetri: V1/V2, A1-A3, Unicode media, Lumetri, Motion keyframes,
    Opacity, Gaussian Blur, Cross Dissolve, Constant Power, clip labels, markers
  - 02_Respeed_And_ClipAttributes: 100%, 50% (Optical Flow), 200%, -100% reverse,
    Interpret Footage FPS=25 + StartTC=10:00:00:00 + PAR, ProRes Alpha V2
  - 03_Nested_Sequences: Sub_B_Roll nested inside parent timeline (trimmed + scaled)
  - 04_Multicam: 3-angle source sequence placed as multicam clip and razored into 3 cuts

Requires PlayerDebugMode=1 for com.adobe.CSXS.12 and no unsaved projects open in Premiere.
"""

import argparse
import json
import shutil
import subprocess
import time
from pathlib import Path

EXT_DIR = Path.home() / "Library/Application Support/Adobe/CEP/extensions/com.aagedal.nlecorpus"
BRIDGE_DIR = Path("/tmp/aagedal_ppro_bridge")

MANIFEST_XML = """<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<ExtensionManifest Version="6.0" ExtensionBundleId="com.aagedal.nlecorpus" ExtensionBundleVersion="0.1.0" ExtensionBundleName="Aagedal NLE Corpus Bridge" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <ExtensionList>
    <Extension Id="com.aagedal.nlecorpus.runner" Version="0.1.0" />
  </ExtensionList>
  <ExecutionEnvironment>
    <HostList>
      <Host Name="PPRO" Version="[0.0,99.9]" />
    </HostList>
    <LocaleList><Locale Code="All" /></LocaleList>
    <RequiredRuntimeList><RequiredRuntime Name="CSXS" Version="9.0" /></RequiredRuntimeList>
  </ExecutionEnvironment>
  <DispatchInfoList>
    <Extension Id="com.aagedal.nlecorpus.runner">
      <DispatchInfo>
        <Resources>
          <MainPath>./index.html</MainPath>
          <CEFCommandLine>
            <Parameter>--enable-nodejs</Parameter>
            <Parameter>--mixed-context</Parameter>
          </CEFCommandLine>
        </Resources>
        <Lifecycle>
          <AutoVisible>false</AutoVisible>
          <StartOn>
            <Event>com.adobe.csxs.events.ApplicationActivate</Event>
            <Event>com.adobe.csxs.events.ApplicationInitialized</Event>
            <Event>applicationActivate</Event>
          </StartOn>
        </Lifecycle>
        <UI><Type>Custom</Type><Geometry><Size><Width>200</Width><Height>100</Height></Size></Geometry></UI>
      </DispatchInfo>
    </Extension>
  </DispatchInfoList>
</ExtensionManifest>
"""

INDEX_HTML = """<!doctype html>
<html><head><meta charset="utf-8"></head><body><script>
const fs = require("fs");
const path = require("path");
const BRIDGE = "/tmp/aagedal_ppro_bridge";
fs.mkdirSync(BRIDGE, { recursive: true });
fs.writeFileSync(path.join(BRIDGE, "ready.txt"), String(Date.now()));
let busy = false;
setInterval(() => {
  if (busy) return;
  const jobFile = path.join(BRIDGE, "job.jsx");
  if (!fs.existsSync(jobFile)) return;
  busy = true;
  let script = "";
  try { script = fs.readFileSync(jobFile, "utf8"); fs.unlinkSync(jobFile); } catch (e) { busy = false; return; }
  const doneFile = path.join(BRIDGE, "result.txt");
  try {
    window.__adobe_cep__.evalScript(script, (res) => {
      try { fs.writeFileSync(doneFile, String(res === undefined ? "" : res)); } finally { busy = false; }
    });
  } catch (e) {
    fs.writeFileSync(doneFile, "BRIDGE_ERR: " + String(e));
    busy = false;
  }
}, 300);
</script></body></html>
"""

JSX_TEMPLATE = r"""
(function() {
  var log = [];
  function info(s) { log.push(s); }
  function safe(label, fn) { try { return fn(); } catch(e) { info("ERR[" + label + "]: " + e); return null; } }
  try {
    app.enableQE();
    var outDir = __OUT_DIR__;
    var mediaDir = __MEDIA_DIR__;
    var projPath = outDir + "/Premiere2026_Corpus.prproj";
    var f = new File(projPath);
    if (f.exists) f.remove();

    app.newProject(projPath);
    var p = app.project;
    var root = p.rootItem;

    var footageBin = root.createBin("01_Footage");
    var camBin = footageBin.createBin("Cameras");
    var audioBin = footageBin.createBin("Audio");
    var attrBin = footageBin.createBin("Modified_Attributes");
    var seqBin = root.createBin("02_Sequences");
    var nestBin = root.createBin("03_Nested_Multicam");

    p.importFiles([
      mediaDir + "/TimeCode_HEVC.mov",
      mediaDir + "/ShortPlantHDR.mp4",
      mediaDir + "/HDR_MultiAudioTrack.mov",
      mediaDir + "/Plant_TV_ProRes_Alpha.mov",
      mediaDir + "/ÖgonKrålærepø_HapoelBersheva.mov"
    ], true, camBin, false);
    p.importFiles([mediaDir + "/TV 2 Nyhetsbed 1.mp3"], true, audioBin, false);
    p.importFiles([mediaDir + "/ShortPlantHDR_av1_h265_tv.mov"], true, attrBin, false);

    function findChild(bin, substr) {
      for (var i = 0; i < bin.children.numItems; i++) {
        if (bin.children[i].name.indexOf(substr) >= 0) return bin.children[i];
      }
      return null;
    }

    var tcClip = findChild(camBin, "TimeCode_HEVC");
    var plantClip = findChild(camBin, "ShortPlantHDR");
    var hdrClip = findChild(camBin, "HDR_MultiAudioTrack");
    var alphaClip = findChild(camBin, "Plant_TV_ProRes_Alpha");
    var unicodeClip = findChild(camBin, "HapoelBersheva");
    var musicClip = findChild(audioBin, "Nyhetsbed");
    var plantMod = findChild(attrBin, "ShortPlantHDR_av1");

    if (tcClip) tcClip.setColorLabel(1);
    if (plantClip) plantClip.setColorLabel(3);
    if (hdrClip) hdrClip.setColorLabel(5);

    var TICKS_PER_SEC = 254016000000;
    function setInOut(item, inSec, outSec) {
      item.setInPoint(String(Math.round(inSec * TICKS_PER_SEC)), 4);
      item.setOutPoint(String(Math.round(outSec * TICKS_PER_SEC)), 4);
    }

    if (plantMod) {
      safe("plantMod_attrs", function() {
        plantMod.setOverrideFrameRate(25.0);
        plantMod.setOverridePixelAspectRatio(1, 1);
        plantMod.setStartTime(String(36000 * TICKS_PER_SEC));
        plantMod.setColorLabel(7);
      });
    }

    // 1. Multi-track + Motion + Lumetri + Gaussian Blur + Transitions + Markers
    setInOut(tcClip, 1.0, 6.0);
    var seq1 = p.createNewSequenceFromClips("01_MultiTrack_FX_Motion_Lumetri", [tcClip], seqBin);
    p.openSequence(seq1.sequenceID);
    seq1.setZeroPoint(String(3600 * TICKS_PER_SEC));
    setInOut(plantClip, 2.0, 8.0);
    seq1.videoTracks[0].overwriteClip(plantClip, 5.0);
    if (unicodeClip) { setInOut(unicodeClip, 0.0, 4.0); seq1.videoTracks[0].overwriteClip(unicodeClip, 11.0); }
    var qseq1 = qe.project.getActiveSequence();
    qseq1.addTracks(1, seq1.videoTracks.numTracks, 2, 1, seq1.audioTracks.numTracks);
    setInOut(hdrClip, 4.0, 9.0);
    seq1.videoTracks[1].overwriteClip(hdrClip, 3.0);
    if (musicClip) { setInOut(musicClip, 0.0, 15.0); seq1.audioTracks[Math.min(2, seq1.audioTracks.numTracks - 1)].overwriteClip(musicClip, 0.0); }

    var c2 = seq1.videoTracks[0].clips[1];
    if (c2) {
      safe("seq1_motion", function() {
        for (var ci = 0; ci < c2.components.numItems; ci++) {
          var comp = c2.components[ci];
          if (comp.matchName === "AE.ADBE Motion") {
            for (var pi = 0; pi < comp.properties.numItems; pi++) {
              var prop = comp.properties[pi];
              if (prop.displayName === "Scale") {
                prop.setTimeVarying(true);
                prop.addKey(c2.inPoint.seconds); prop.setValueAtKey(c2.inPoint.seconds, 120.0, true);
                prop.addKey(c2.outPoint.seconds); prop.setValueAtKey(c2.outPoint.seconds, 145.0, true);
              } else if (prop.displayName === "Rotation") { prop.setValue(5.5, true); }
            }
          }
        }
      });
      var qv0 = qseq1.getVideoTrackAt(0);
      safe("seq1_fx", function() {
        var qc2 = qv0.getItemAt(1);
        var lumFx = qe.project.getVideoEffectByName("Lumetri Color");
        var blurFx = qe.project.getVideoEffectByName("Gaussian Blur");
        if (qc2 && lumFx) qc2.addVideoEffect(lumFx);
        if (qc2 && blurFx) qc2.addVideoEffect(blurFx);
        for (var ci2 = 0; ci2 < c2.components.numItems; ci2++) {
          var lcomp = c2.components[ci2];
          if (lcomp.matchName.toLowerCase().indexOf("lumetri") >= 0) {
            for (var lp = 0; lp < lcomp.properties.numItems; lp++) {
              var lprop = lcomp.properties[lp];
              try {
                if (lprop.displayName === "Exposure") lprop.setValue(0.6, true);
                else if (lprop.displayName === "Contrast") lprop.setValue(20.0, true);
                else if (lprop.displayName === "Saturation") lprop.setValue(115.0, true);
                else if (lprop.displayName === "Temperature") lprop.setValue(10.0, true);
              } catch(e) {}
            }
          }
        }
        var dissolve = qe.project.getVideoTransitionByName("Cross Dissolve");
        if (qv0.getItemAt(0) && dissolve) qv0.getItemAt(0).addTransition(dissolve, false, "00:00:01:00");
      });
    }
    safe("seq1_v2_a1_markers", function() {
      var cv2 = seq1.videoTracks[1].clips[0];
      if (cv2) {
        for (var vi = 0; vi < cv2.components.numItems; vi++) {
          var vc = cv2.components[vi];
          if (vc.matchName === "AE.ADBE Motion") {
            for (var vp = 0; vp < vc.properties.numItems; vp++) if (vc.properties[vp].displayName === "Scale") vc.properties[vp].setValue(45.0, true);
          } else if (vc.matchName === "AE.ADBE Opacity") {
            for (var op = 0; op < vc.properties.numItems; op++) if (vc.properties[op].displayName === "Opacity") vc.properties[op].setValue(85.0, true);
          }
        }
      }
      var qa0 = qseq1.getAudioTrackAt(0);
      var cpTr = qe.project.getAudioTransitionByName("Constant Power");
      if (qa0 && qa0.getItemAt(0) && cpTr) qa0.getItemAt(0).addTransition(cpTr, false, "00:00:00:20");
      var m1 = seq1.markers.createMarker(2.0); m1.name = "Intro Cut"; m1.comments = "Check V1/V2 overlap & Lumetri"; m1.end = 2.5;
      var m2 = seq1.markers.createMarker(8.0); m2.name = "Blur Check"; m2.comments = "Gaussian Blur + keyframed Motion"; m2.setColorByIndex(2);
    });

    // 2. Respeed & Clip Attributes
    setInOut(tcClip, 0.0, 4.0);
    var seq2 = p.createNewSequenceFromClips("02_Respeed_And_ClipAttributes", [tcClip], seqBin);
    p.openSequence(seq2.sequenceID);
    seq2.setZeroPoint(String(3600 * TICKS_PER_SEC));
    setInOut(tcClip, 4.0, 8.0); seq2.videoTracks[0].overwriteClip(tcClip, 4.0);
    setInOut(tcClip, 8.0, 12.0); seq2.videoTracks[0].overwriteClip(tcClip, 8.0);
    setInOut(tcClip, 12.0, 16.0); seq2.videoTracks[0].overwriteClip(tcClip, 12.0);
    if (plantMod) { setInOut(plantMod, 0.0, 4.0); seq2.videoTracks[0].overwriteClip(plantMod, 16.0); }
    var qseq2 = qe.project.getActiveSequence();
    qseq2.addTracks(1, seq2.videoTracks.numTracks, 0, 1, seq2.audioTracks.numTracks);
    if (alphaClip) { setInOut(alphaClip, 0.0, 4.0); seq2.videoTracks[1].overwriteClip(alphaClip, 16.0); }
    safe("seq2_speeds", function() {
      var qv2 = qseq2.getVideoTrackAt(0);
      qv2.getItemAt(1).setSpeed(0.5, "", false, false, false);
      qv2.getItemAt(1).setTimeInterpolationType(2);
      qv2.getItemAt(2).setSpeed(2.0, "", false, false, false);
      qv2.getItemAt(3).setSpeed(1.0, "", true, false, false);
    });

    // 3. Nested Sequences
    setInOut(plantClip, 0.0, 3.0);
    var subSeq = p.createNewSequenceFromClips("Sub_B_Roll", [plantClip], nestBin);
    p.openSequence(subSeq.sequenceID);
    setInOut(hdrClip, 0.0, 3.0); subSeq.videoTracks[0].overwriteClip(hdrClip, 3.0);
    var sm = subSeq.markers.createMarker(1.5); sm.name = "Inside Sub_B_Roll"; sm.comments = "Nested sequence internal marker";
    safe("sub_dissolve", function() {
      var qsub = qe.project.getActiveSequence();
      var subDiss = qe.project.getVideoTransitionByName("Cross Dissolve");
      if (qsub.getVideoTrackAt(0).getItemAt(0) && subDiss) qsub.getVideoTrackAt(0).getItemAt(0).addTransition(subDiss, false, "00:00:00:15");
    });
    setInOut(tcClip, 0.0, 4.0);
    var seq3 = p.createNewSequenceFromClips("03_Nested_Sequences", [tcClip], seqBin);
    p.openSequence(seq3.sequenceID);
    seq3.setZeroPoint(String(3600 * TICKS_PER_SEC));
    setInOut(subSeq.projectItem, 0.5, 5.5);
    seq3.videoTracks[0].overwriteClip(subSeq.projectItem, 4.0);

    // 4. Multicam
    setInOut(tcClip, 0.0, 15.0);
    var mcSource = p.createNewSequenceFromClips("MC_3Cam_Source", [tcClip], nestBin);
    p.openSequence(mcSource.sequenceID);
    var qmcSrc = qe.project.getActiveSequence();
    qmcSrc.addTracks(2, mcSource.videoTracks.numTracks, 2, 1, mcSource.audioTracks.numTracks);
    setInOut(plantClip, 0.0, 15.0); mcSource.videoTracks[1].overwriteClip(plantClip, 0.0);
    setInOut(hdrClip, 0.0, 15.0); mcSource.videoTracks[2].overwriteClip(hdrClip, 0.0);

    setInOut(mcSource.projectItem, 0.0, 15.0);
    var seq4 = p.createNewSequenceFromClips("04_Multicam", [mcSource.projectItem], seqBin);
    p.openSequence(seq4.sequenceID);
    seq4.setZeroPoint(String(3600 * TICKS_PER_SEC));
    safe("seq4_multicam", function() {
      var qseq4 = qe.project.getActiveSequence();
      qseq4.getVideoTrackAt(0).getItemAt(0).setMulticam(true);
      qseq4.razor("00:00:05:00");
      qseq4.razor("00:00:10:00");
    });

    p.save();
    var exportSeqs = [
      [seq1, "01_MultiTrack_FX_Motion_Lumetri_fcp7.xml"],
      [seq2, "02_Respeed_And_ClipAttributes_fcp7.xml"],
      [seq3, "03_Nested_Sequences_fcp7.xml"],
      [seq4, "04_Multicam_fcp7.xml"]
    ];
    for (var ei = 0; ei < exportSeqs.length; ei++) {
      exportSeqs[ei][0].exportAsFinalCutProXML(outDir + "/" + exportSeqs[ei][1]);
    }
    p.closeDocument(0, 0);
    info("DONE: " + projPath);
  } catch(e) {
    info("FATAL: " + e);
  }
  return log.join("\n");
})();
"""


def main():
    parser = argparse.ArgumentParser(description="Generate Adobe Premiere Pro 2026 reference corpus")
    parser.add_argument("--media-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--app-path", type=str, default="/Applications/Adobe Premiere Pro 2026/Adobe Premiere Pro 2026.app")
    args = parser.parse_args()

    args.output_dir.mkdir(parents=True, exist_ok=True)
    if EXT_DIR.exists():
        shutil.rmtree(EXT_DIR)
    if BRIDGE_DIR.exists():
        shutil.rmtree(BRIDGE_DIR)

    try:
        (EXT_DIR / "CSXS").mkdir(parents=True, exist_ok=True)
        BRIDGE_DIR.mkdir(parents=True, exist_ok=True)
        (EXT_DIR / "CSXS" / "manifest.xml").write_text(MANIFEST_XML)
        (EXT_DIR / "index.html").write_text(INDEX_HTML)

        subprocess.run(["killall", "Adobe Premiere Pro 2026"], stderr=subprocess.DEVNULL)
        time.sleep(2.0)
        subprocess.run(["open", "-g", "-a", args.app_path], check=True)

        ready = BRIDGE_DIR / "ready.txt"
        for _ in range(60):
            if ready.exists():
                break
            time.sleep(1.0)
        else:
            raise RuntimeError("Timed out waiting for Premiere Pro CEP runner")

        jsx = (
            JSX_TEMPLATE
            .replace("__OUT_DIR__", json.dumps(str(args.output_dir)))
            .replace("__MEDIA_DIR__", json.dumps(str(args.media_dir)))
        )
        res = BRIDGE_DIR / "result.txt"
        if res.exists():
            res.unlink()
        (BRIDGE_DIR / "job.jsx").write_text(jsx)
        for _ in range(120):
            if res.exists():
                print(res.read_text())
                return
            time.sleep(0.5)
        raise RuntimeError("Timed out waiting for ExtendScript completion")
    finally:
        if EXT_DIR.exists():
            shutil.rmtree(EXT_DIR)
        if BRIDGE_DIR.exists():
            shutil.rmtree(BRIDGE_DIR)


if __name__ == "__main__":
    main()
