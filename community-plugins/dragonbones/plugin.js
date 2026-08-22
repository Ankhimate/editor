// DragonBones JSON community plugin for Ankhimate.
// Clean-room conversion based on observed files and Ankhimate's public schema.
(function () {
  "use strict";
  const num = (value, fallback = 0) => typeof value === "number" && Number.isFinite(value) ? value : fallback;
  const list = (value) => Array.isArray(value) ? value : [];
  const object = (value) => value && typeof value === "object" && !Array.isArray(value) ? value : {};
  const linear = () => ({ curve: "linear" });
  const stepped = () => ({ curve: "stepped" });

  function frameInterp(frame, report, where) {
    if (Array.isArray(frame.curve) && frame.curve.length >= 4) {
      const ox = num(frame.curve[0]), oy = num(frame.curve[1]);
      const ix = num(frame.curve[2]), iy = num(frame.curve[3]);
      const outX = Math.max(0, Math.min(1, ox));
      const inX = Math.max(0, Math.min(1, ix));
      if (outX !== ox || inX !== ix) report.lossy.push({ what: "curve", where,
        detail: "a time handle outside the segment was clamped so the curve stays samplable" });
      return { curve: "bezier", handles: [outX, oy, inX, iy] };
    }
    if (frame.tweenEasing === 0) return linear();
    if (typeof frame.tweenEasing === "number") {
      const amount = Math.max(-1, Math.min(1, frame.tweenEasing));
      return amount >= 0
        ? { curve: "bezier", handles: [0, 0, 1 - amount * 0.5, 1] }
        : { curve: "bezier", handles: [-amount * 0.5, 0, 1, 1] };
    }
    return stepped();
  }

  function frameKeys(frames, fps, value, report, where) {
    let elapsed = 0;
    return list(frames).map((frame, index, all) => {
      const key = { time: elapsed / fps, value: value(frame) };
      Object.assign(key, index === 0 ? linear() : frameInterp(all[index - 1], report, where));
      elapsed += num(frame.duration, 1);
      return key;
    });
  }
  const flat = (keys, value) => keys.every((key) => Math.abs(key.value - value) < 1e-6);

  function atlasFor(fileName) {
    const stem = String(fileName || "").replace(/_ske\.json$/i, "").replace(/\.json$/i, "");
    const text = ankhimate.sidecar(`${stem}_tex.json`);
    if (!text) return null;
    let atlas;
    try { atlas = JSON.parse(text); } catch (_) { return null; }
    const regions = {};
    for (const item of list(atlas.SubTexture)) regions[item.name] = {
      x: num(item.x), y: num(item.y), width: num(item.width), height: num(item.height),
      rotated: Boolean(item.rotated),
    };
    return { page: atlas.imagePath || `${stem}_tex.png`, regions,
      bytes: ankhimate.sidecarBytes(atlas.imagePath || `${stem}_tex.png`) };
  }

  function convert(text, fileName) {
    let doc;
    try { doc = JSON.parse(text); } catch (_) { throw new Error("not a DragonBones skeleton"); }
    const armatures = list(doc.armature);
    if (armatures.length === 0) throw new Error("not a DragonBones skeleton");
    const armature = armatures[0];
    const fps = Math.max(1, num(armature.frameRate, num(doc.frameRate, 24)));
    const report = { dangling: [], lossy: [] };
    const project = {
      version: 3, name: String(doc.name || fileName || "imported").replace(/_ske\.json$/i, "").replace(/\.json$/i, ""),
      fps: Math.round(fps), assets: [], bones: [], slots: [], draw_order: [], skins: [],
      default_skin: "default", constraints: [], constraint_order: [], animations: [],
    };
    const images = {};
    const boneByName = {};
    for (const bone of list(armature.bone)) {
      const transform = object(bone.transform);
      const converted = {
        name: String(bone.name || "bone"), parent: String(bone.parent || ""),
        length: Math.max(1, num(bone.length)), tx: num(transform.x), ty: -num(transform.y),
        rotation: -num(transform.skY), sx: num(transform.scX, 1), sy: num(transform.scY, 1),
        shear_x: 0, shear_y: -(num(transform.skX) - num(transform.skY)),
        inherit_rotation: bone.inheritRotation !== false,
        inherit_scale: bone.inheritScale !== false,
        inherit_reflect: bone.inheritReflection !== false,
      };
      project.bones.push(converted); boneByName[converted.name] = converted;
    }

    const displayIndex = {};
    const slotByName = {};
    for (const slot of list(armature.slot)) {
      if (!boneByName[slot.parent]) {
        report.dangling.push({ what: "dragonbones slot parent", name: String(slot.parent || "") });
        continue;
      }
      const converted = { name: String(slot.name || "slot"), bone: slot.parent,
        attachment: null, color: [1,1,1,1], dark_color: null, blend_mode: "normal" };
      project.slots.push(converted); project.draw_order.push(converted.name);
      slotByName[converted.name] = converted;
      displayIndex[converted.name] = Math.trunc(num(slot.displayIndex));
    }

    const atlas = atlasFor(fileName);
    const decoded = new Set();
    function asset(regionName) {
      if (decoded.has(regionName)) return project.assets.find((entry) => entry.name === regionName);
      const region = atlas && atlas.regions[regionName];
      let bytes = null, width = 0, height = 0;
      if (region && atlas.bytes) {
        bytes = ankhimate.cropImage(atlas.bytes, {
          x: region.x, y: region.y,
          width: region.rotated ? region.height : region.width,
          height: region.rotated ? region.width : region.height,
          quarter_turns_clockwise: region.rotated ? 3 : 0,
        });
        width = region.width; height = region.height;
      } else {
        bytes = ankhimate.sidecarBytes(`${regionName}.png`);
        if (bytes) {
          const info = ankhimate.imageInfo(bytes);
          width = info.width; height = info.height;
        }
      }
      if (!bytes) {
        report.dangling.push({ what: "dragonbones region", name: regionName });
        return null;
      }
      const result = { name: regionName, file: `${regionName}.png`, width, height };
      project.assets.push(result); images[regionName] = bytes; decoded.add(regionName);
      return result;
    }

    const displayLists = {};
    const foldedArmatures = new Set();
    const skin = list(armature.skin)[0];
    const resultSkin = { name: String((skin && skin.name) || "default"), entries: [] };
    for (const entry of list(skin && skin.slot)) {
      if (!slotByName[entry.name]) continue;
      const names = [];
      for (const display of list(entry.display)) {
        const displayName = display.name == null ? null : String(display.name);
        names.push(displayName);
        if (!displayName) continue;
        const regionName = String(display.path || displayName);
        const kind = String(display.type || "image");
        if (kind === "image") {
          const found = asset(regionName); if (!found) continue;
          const transform = object(display.transform), pivot = object(display.pivot);
          resultSkin.entries.push({ slot: entry.name, name: displayName, attachment: {
            type: "region", texture: regionName, offset_x: num(transform.x), offset_y: -num(transform.y),
            rotation: -num(transform.skY), scale_x: num(transform.scX, 1), scale_y: num(transform.scY, 1),
            width: found.width, height: found.height, uv: [0,0,1,1],
            pivot_x: num(pivot.x, 0.5), pivot_y: 1 - num(pivot.y, 0.5),
          }});
        } else if (kind === "mesh") {
          const found = asset(regionName); if (!found) continue;
          const vertices = list(display.vertices).map((value, index) => index % 2 ? -num(value) : num(value));
          const triangles = list(display.triangles).map(Number);
          if (!vertices.length || !triangles.length) {
            report.lossy.push({ what: "attachment", where: `${entry.name}/${displayName}`,
              detail: "a mesh with no vertices or no triangles was skipped" });
            continue;
          }
          if (display.weights != null) report.lossy.push({ what: "attachment", where: `${entry.name}/${displayName}`,
            detail: "a weighted mesh imported without its weights, so it follows its slot's bone rigidly" });
          resultSkin.entries.push({ slot: entry.name, name: displayName, attachment: {
            type: "mesh", texture: regionName, vertices, uvs: list(display.uvs).map(Number),
            triangles, weights: [],
          }});
        } else if (kind === "armature") {
          const nested = armatures.find((candidate) => candidate.name === displayName);
          if (!nested) {
            report.dangling.push({ what: "dragonbones armature", name: displayName });
            continue;
          }
          const host = object(display.transform);
          const nestedBone = object((list(nested.bone)[0] || {}).transform);
          const boneRotation = -num(nestedBone.skY);
          const angle = boneRotation * Math.PI / 180;
          const cos = Math.cos(angle), sin = Math.sin(angle);
          let firstFolded = null;
          for (const nestedSlot of list((list(nested.skin)[0] || {}).slot)) {
            const frames = [];
            let first = null;
            for (const nestedDisplay of list(nestedSlot.display)) {
              if (String(nestedDisplay.type || "image") !== "image" || !nestedDisplay.name) continue;
              const regionName = String(nestedDisplay.path || nestedDisplay.name);
              const found = asset(regionName); if (!found) continue;
              frames.push(regionName);
              if (!first) first = { display: nestedDisplay, asset: found };
            }
            if (!first) continue;
            const transform = object(first.display.transform), pivot = object(first.display.pivot);
            const dx = num(transform.x), dy = -num(transform.y);
            const rotatedX = (dx * cos - dy * sin) * num(nestedBone.scX, 1);
            const rotatedY = (dx * sin + dy * cos) * num(nestedBone.scY, 1);
            const attachmentName = `${displayName}#${names.length - 1}`;
            resultSkin.entries.push({ slot: entry.name, name: attachmentName, attachment: {
              type: "region", texture: frames[0],
              offset_x: num(host.x) + (num(nestedBone.x) + rotatedX) * num(host.scX, 1),
              offset_y: -num(host.y) + (-num(nestedBone.y) + rotatedY) * num(host.scY, 1),
              rotation: boneRotation - num(transform.skY),
              scale_x: num(host.scX, 1) * num(nestedBone.scX, 1) * num(transform.scX, 1),
              scale_y: num(host.scY, 1) * num(nestedBone.scY, 1) * num(transform.scY, 1),
              width: first.asset.width, height: first.asset.height, uv: [0,0,1,1],
              pivot_x: num(pivot.x, 0.5), pivot_y: 1 - num(pivot.y, 0.5),
              sequence: frames.length > 1 ? {
                frames, fps: Math.max(1, num(nested.frameRate, fps)), mode: "loop", setup_index: 0,
              } : null,
            }});
            if (!firstFolded) firstFolded = attachmentName;
          }
          if (firstFolded) {
            names[names.length - 1] = firstFolded;
            foldedArmatures.add(displayName);
          } else {
            report.lossy.push({ what: "attachment", where: `${entry.name}/${displayName}`,
              detail: "a nested armature held nothing this reader could fold in" });
          }
        } else {
          report.lossy.push({ what: "attachment", where: `${entry.name}/${displayName}`,
            detail: kind === "boundingBox" ? "a bounding box display is not read yet" : "an unrecognised display type was skipped" });
        }
      }
      displayLists[entry.name] = names;
      const index = displayIndex[entry.name] ?? 0;
      if (index >= 0 && names[index]) slotByName[entry.name].attachment = names[index];
    }
    project.skins.push(resultSkin); project.default_skin = resultSkin.name;

    for (const extra of armatures.slice(1)) if (!foldedArmatures.has(extra.name)) {
      report.lossy.push({ what: "armature", where: String(extra.name || "unnamed"),
        detail: "no display referenced this armature, and a document holds one skeleton" });
    }

    for (const ik of list(armature.ik)) {
      const name = String(ik.name || "ik");
      if (!boneByName[ik.bone] || !boneByName[ik.target]) {
        report.dangling.push({ what: "dragonbones ik bone", name: String(ik.bone || "") });
        continue;
      }
      const chain = [ik.bone];
      let current = boneByName[ik.bone];
      for (let count = 0; count < Math.max(0, Math.trunc(num(ik.chain))); count++) {
        current = current && boneByName[current.parent];
        if (!current) break;
        chain.push(current.name);
      }
      chain.reverse();
      project.constraints.push({ name, type: "ik", target: ik.target, bones: chain,
        bend_direction: ik.bendPositive === false ? 1 : -1,
        mix: Math.max(0, Math.min(1, num(ik.weight, 100) / 100)), softness: 0,
        stretch: false, stretch_limit: 1.1, stiffness: 0 });
      project.constraint_order.push(name);
    }

    for (const animation of list(armature.animation)) {
      const name = String(animation.name || "animation");
      const result = { name, duration: num(animation.duration) / fps, looping: false, timelines: [], events: [] };
      for (const track of list(animation.bone)) {
        if (!boneByName[track.name]) {
          report.dangling.push({ what: "dragonbones animated bone", name: String(track.name || "") });
          continue;
        }
        const where = `${name}/${track.name}`;
        if (Array.isArray(track.translateFrame)) {
          for (const [axis, sign] of [["x", 1], ["y", -1]]) {
            const keys = frameKeys(track.translateFrame, fps, (frame) => sign * num(frame[axis]), report, where);
            if (!flat(keys, 0)) result.timelines.push({ kind: "bone_translate", bone: track.name, axis, keys });
          }
        }
        if (Array.isArray(track.rotateFrame)) {
          const keys = frameKeys(track.rotateFrame, fps, (frame) => -num(frame.rotate), report, where);
          if (!flat(keys, 0)) result.timelines.push({ kind: "bone_rotate", bone: track.name, keys });
        }
        if (Array.isArray(track.scaleFrame)) {
          for (const axis of ["x", "y"]) {
            const keys = frameKeys(track.scaleFrame, fps, (frame) => num(frame[axis], 1), report, where);
            if (!flat(keys, 1)) result.timelines.push({ kind: "bone_scale", bone: track.name, axis, keys });
          }
        }
      }
      for (const track of list(animation.slot)) {
        if (!slotByName[track.name]) {
          report.dangling.push({ what: "dragonbones animated slot", name: String(track.name || "") });
          continue;
        }
        const frames = list(track.displayFrame), names = displayLists[track.name] || [];
        let elapsed = 0;
        const keys = frames.map((frame) => {
          const index = Math.trunc(num(frame.value));
          const key = { time: elapsed / fps, value: index < 0 ? null : (names[index] ?? null) };
          elapsed += num(frame.duration, 1); return key;
        });
        if (keys.some((key, index) => index && key.value !== keys[index - 1].value)) {
          result.timelines.push({ kind: "slot_attachment", slot: track.name, keys });
        }
      }
      if (list(animation.timeline).length) report.lossy.push({ what: "timeline", where: name,
        detail: "a DragonBones 5.6 generic timeline (numeric `type` codes) is not read yet" });
      project.animations.push(result);
    }

    ankhimate.importProject(project, images, report);
  }

  ankhimate.registerImporter({
    id: "import.dragonbones", label: "DragonBones (community)", extensions: ["json"],
    canRead(text) { try { return list(JSON.parse(text).armature).length > 0; } catch (_) { return false; } },
    read: convert,
  });
})();
