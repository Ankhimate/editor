// Spine JSON community plugin for Ankhimate.
// Clean-room conversion from the documented Ankhimate schema and observed
// importer behavior. No Spine runtime or editor source is used.
(function () {
  "use strict";

  const num = (value, fallback = 0) =>
    typeof value === "number" && Number.isFinite(value) ? value : fallback;
  const list = (value) => Array.isArray(value) ? value : [];
  const object = (value) => value && typeof value === "object" && !Array.isArray(value)
    ? value : {};
  const hex = (value, fallback = [1, 1, 1, 1]) => {
    if (typeof value !== "string" || value.length !== 8) return fallback.slice();
    const out = [0, 2, 4, 6].map((at) => parseInt(value.slice(at, at + 2), 16) / 255);
    return out.every(Number.isFinite) ? out : fallback.slice();
  };
  const linear = () => ({ curve: "linear" });
  const stepped = () => ({ curve: "stepped" });

  function inherit(mode) {
    switch (mode || "normal") {
      case "onlyTranslation": return [false, false, false];
      case "noRotationOrReflection": return [false, true, false];
      case "noScale": return [true, false, true];
      case "noScaleOrReflection": return [true, false, false];
      default: return [true, true, true];
    }
  }

  function interp(curve, t0, v0, t1, v1, channel, report, where) {
    if (curve === "stepped") return stepped();
    if (!Array.isArray(curve)) return linear();
    const at = (index) => num(curve[channel * 4 + index], 0);
    const dt = t1 - t0;
    const dv = v1 - v0;
    const nx = (x) => Math.abs(dt) < 1e-6 ? 0 : (x - t0) / dt;
    const ny = (y) => Math.abs(dv) < 1e-6 ? 0 : (y - v0) / dv;
    const rawOutX = nx(at(0));
    const rawInX = nx(at(2));
    const outX = Math.max(0, Math.min(1, rawOutX));
    const inX = Math.max(0, Math.min(1, rawInX));
    if (Math.abs(rawOutX - outX) > 1e-4 || Math.abs(rawInX - inX) > 1e-4) {
      report.lossy.push({
        what: "curve", where,
        detail: "a handle reached outside the segment in time and was clamped to it; an easing that doubles back in time cannot be sampled",
      });
    }
    const finite = (v) => Number.isFinite(v) ? v : 0;
    return {
      curve: "bezier",
      handles: [outX, finite(ny(at(1))), inX, finite(ny(at(3)))],
    };
  }

  function scalarKeys(frames, channel, value, report, where) {
    return list(frames).map((frame, index, all) => {
      const key = { time: num(frame.time), value: value(frame) };
      Object.assign(key, index === 0 ? linear() : interp(
        all[index - 1].curve,
        num(all[index - 1].time), value(all[index - 1]),
        key.time, key.value, channel, report, where,
      ));
      return key;
    });
  }

  function colorKeys(frames, report, where) {
    return list(frames).map((frame, index, all) => {
      const value = hex(frame.color);
      const key = { time: num(frame.time), value };
      Object.assign(key, index === 0 ? linear() : interp(
        all[index - 1].curve,
        num(all[index - 1].time), hex(all[index - 1].color)[3],
        key.time, value[3], 0, report, where,
      ));
      return key;
    });
  }

  function parseAtlas(text) {
    let page = "";
    const regions = {};
    let current = null;
    let bounds = null;
    let rotated = false;
    const flush = () => {
      if (current && bounds) regions[current] = { ...bounds, rotated };
    };
    for (const raw of String(text || "").split(/\r?\n/)) {
      const line = raw.trim();
      if (!line) continue;
      const indented = raw.startsWith("\t") || raw.startsWith("  ");
      if (!indented) {
        if (!page && line.endsWith(".png")) { page = line; continue; }
        flush(); current = line; bounds = null; rotated = false; continue;
      }
      const colon = line.indexOf(":");
      if (colon < 0) continue;
      const key = line.slice(0, colon).trim();
      const value = line.slice(colon + 1).trim();
      const values = value.split(",").map((v) => Number(v.trim()));
      if (key === "bounds" && values.length === 4 && values.every(Number.isFinite)) {
        bounds = { x: values[0], y: values[1], width: values[2], height: values[3] };
      } else if (key === "rotate") {
        rotated = value === "90" || value === "true";
      }
    }
    flush();
    return { page, regions };
  }

  const radians = (degrees) => degrees * Math.PI / 180;
  function affine(transform) {
    const x = radians(transform.rotation + transform.shear_x);
    const y = radians(transform.rotation + 90 + transform.shear_y);
    return { a: Math.cos(x) * transform.sx, b: Math.sin(x) * transform.sx,
      c: Math.cos(y) * transform.sy, d: Math.sin(y) * transform.sy,
      tx: transform.tx, ty: transform.ty };
  }
  function multiply(left, right) {
    return {
      a: left.a * right.a + left.c * right.b,
      b: left.b * right.a + left.d * right.b,
      c: left.a * right.c + left.c * right.d,
      d: left.b * right.c + left.d * right.d,
      tx: left.a * right.tx + left.c * right.ty + left.tx,
      ty: left.b * right.tx + left.d * right.ty + left.ty,
    };
  }
  const point = (matrix, x, y) => [
    matrix.a * x + matrix.c * y + matrix.tx,
    matrix.b * x + matrix.d * y + matrix.ty,
  ];
  function inverse(matrix) {
    const determinant = matrix.a * matrix.d - matrix.b * matrix.c;
    if (Math.abs(determinant) < 1e-12) return null;
    const a = matrix.d / determinant, b = -matrix.b / determinant;
    const c = -matrix.c / determinant, d = matrix.a / determinant;
    return { a, b, c, d, tx: -(a * matrix.tx + c * matrix.ty),
      ty: -(b * matrix.tx + d * matrix.ty) };
  }
  function decompose(matrix) {
    const rotation = Math.atan2(matrix.b, matrix.a);
    const sx = Math.hypot(matrix.a, matrix.b);
    const determinant = matrix.a * matrix.d - matrix.b * matrix.c;
    const sy = Math.hypot(matrix.c, matrix.d) * (determinant < 0 ? -1 : 1);
    const yAngle = sy < 0 ? Math.atan2(-matrix.d, -matrix.c) : Math.atan2(matrix.d, matrix.c);
    let shearY = yAngle - rotation - Math.PI / 2;
    while (shearY <= -Math.PI) shearY += Math.PI * 2;
    while (shearY > Math.PI) shearY -= Math.PI * 2;
    return { rotation, sx, sy, shearY };
  }
  function childWorld(parent, local) {
    const own = affine(local);
    if (!parent) return own;
    if (local.inherit_rotation && local.inherit_scale && local.inherit_reflect) {
      return multiply(parent, own);
    }
    const origin = point(parent, local.tx, local.ty);
    const inherited = decompose(parent);
    const effectiveTransform = { tx: 0, ty: 0,
      rotation: local.inherit_rotation ? inherited.rotation * 180 / Math.PI : 0,
      sx: local.inherit_scale ? inherited.sx : 1,
      sy: local.inherit_scale ? inherited.sy : 1,
      shear_x: 0,
      shear_y: local.inherit_scale ? inherited.shearY * 180 / Math.PI : 0,
    };
    if (!local.inherit_reflect && effectiveTransform.sy < 0) {
      effectiveTransform.sy = -effectiveTransform.sy;
    }
    const effective = affine(effectiveTransform);
    const world = multiply(effective, own);
    world.tx = origin[0]; world.ty = origin[1];
    return world;
  }

  function imageSource() {
    const atlasName = ankhimate.sidecars().find((name) => name.endsWith(".atlas"));
    if (!atlasName) return { atlas: null, pageBytes: null };
    const atlas = parseAtlas(ankhimate.sidecar(atlasName));
    return { atlas, pageBytes: atlas.page ? ankhimate.sidecarBytes(atlas.page) : null };
  }

  function convert(text, fileName) {
    let doc;
    try { doc = JSON.parse(text); } catch (_) { throw new Error("not a Spine skeleton"); }
    if (!Array.isArray(doc.bones) || doc.bones.length === 0) {
      throw new Error("not a Spine skeleton");
    }

    const report = { dangling: [], lossy: [] };
    const project = {
      version: 3,
      name: String(fileName || "imported").replace(/\.[^.]+$/, ""),
      fps: 30,
      assets: [], bones: [], slots: [], draw_order: [], skins: [],
      default_skin: "default", constraints: [], constraint_order: [], animations: [],
    };
    const images = {};
    const boneNames = new Set(doc.bones.map((bone) => bone.name));
    for (const bone of doc.bones) {
      const inherited = inherit(bone.inherit);
      project.bones.push({
        name: String(bone.name || "bone"), parent: String(bone.parent || ""),
        length: Math.max(1, num(bone.length)), tx: num(bone.x), ty: num(bone.y),
        rotation: num(bone.rotation), sx: num(bone.scaleX, 1), sy: num(bone.scaleY, 1),
        shear_x: num(bone.shearX), shear_y: num(bone.shearY),
        inherit_rotation: inherited[0], inherit_scale: inherited[1],
        inherit_reflect: inherited[2], color: hex(bone.color, [0, 0.8, 0.8, 0.85]),
      });
    }
    const worlds = {};
    for (const bone of project.bones) {
      worlds[bone.name] = childWorld(bone.parent ? worlds[bone.parent] : null, bone);
    }

    const slotNames = new Set();
    for (const slot of list(doc.slots)) {
      if (!boneNames.has(slot.bone)) {
        report.dangling.push({ what: "spine slot bone", name: String(slot.bone || "") });
        continue;
      }
      const name = String(slot.name || "slot");
      slotNames.add(name);
      project.slots.push({
        name, bone: slot.bone, attachment: slot.attachment ?? null,
        color: hex(slot.color), dark_color: slot.dark ? hex(slot.dark) : null,
        blend_mode: String(slot.blend || "normal"),
      });
      project.draw_order.push(name);
    }

    const source = imageSource();
    const decoded = new Set();
    function ensureAsset(regionName, width, height) {
      if (decoded.has(regionName)) return true;
      let bytes = null;
      let w = Math.max(0, Math.round(num(width)));
      let h = Math.max(0, Math.round(num(height)));
      const region = source.atlas && source.atlas.regions[regionName];
      if (region && source.pageBytes) {
        bytes = ankhimate.cropImage(source.pageBytes, {
          x: region.x, y: region.y,
          width: region.rotated ? region.height : region.width,
          height: region.rotated ? region.width : region.height,
          rotate_clockwise: region.rotated,
        });
        w = region.width; h = region.height;
      } else {
        bytes = ankhimate.sidecarBytes(`images/${regionName}.png`)
          || ankhimate.sidecarBytes(`${regionName}.png`);
      }
      if (!bytes) {
        report.dangling.push({ what: "spine region", name: regionName });
        return false;
      }
      project.assets.push({ name: regionName, file: `${regionName}.png`, width: w, height: h });
      images[regionName] = bytes;
      decoded.add(regionName);
      return true;
    }

    const skinList = Array.isArray(doc.skins) ? doc.skins : [];
    const spineBones = doc.bones;
    const meshInfo = {};
    for (const [skinIndex, skin] of skinList.entries()) {
      const result = { name: String(skin.name || (skinIndex === 0 ? "default" : `skin_${skinIndex}`)), entries: [] };
      const attachmentsBySlot = object(skin.attachments);
      for (const [slotName, attachments] of Object.entries(attachmentsBySlot)) {
        if (!slotNames.has(slotName)) continue;
        for (const [attachmentName, attachment] of Object.entries(object(attachments))) {
          const type = String(attachment.type || "region");
          const regionName = String(attachment.path || attachmentName);
          let converted = null;
          if (type === "region") {
            if (!ensureAsset(regionName, attachment.width, attachment.height)) continue;
            converted = {
              type: "region", texture: regionName,
              offset_x: num(attachment.x), offset_y: num(attachment.y),
              rotation: num(attachment.rotation), scale_x: num(attachment.scaleX, 1),
              scale_y: num(attachment.scaleY, 1), width: num(attachment.width),
              height: num(attachment.height), uv: [0, 0, 1, 1], pivot_x: 0.5, pivot_y: 0.5,
            };
          } else if (type === "mesh") {
            if (!ensureAsset(regionName, attachment.width, attachment.height)) continue;
            const raw = list(attachment.vertices).map(Number);
            const uvs = list(attachment.uvs).map(Number);
            const weighted = raw.length !== uvs.length;
            let vertices = raw;
            let weights = [];
            let influenceCount = 0;
            if (weighted) {
              vertices = [];
              let at = 0;
              const slot = project.slots.find((candidate) => candidate.name === slotName);
              const slotInverse = slot && inverse(worlds[slot.bone]);
              while (at < raw.length) {
                const count = Math.max(0, Math.trunc(raw[at++]));
                let worldX = 0, worldY = 0;
                const influences = [];
                for (let influence = 0; influence < count && at + 3 < raw.length; influence++) {
                  const boneIndex = Math.trunc(raw[at++]);
                  const x = raw[at++], y = raw[at++], weight = raw[at++];
                  const sourceBone = spineBones[boneIndex];
                  const matrix = sourceBone && worlds[sourceBone.name];
                  if (!matrix) continue;
                  const placed = point(matrix, x, y);
                  worldX += placed[0] * weight; worldY += placed[1] * weight;
                  influences.push([sourceBone.name, weight]);
                  influenceCount++;
                }
                const local = slotInverse ? point(slotInverse, worldX, worldY) : [worldX, worldY];
                vertices.push(local[0], local[1]);
                weights.push(influences);
              }
            }
            converted = {
              type: "mesh", texture: regionName, vertices, uvs,
              triangles: list(attachment.triangles).map(Number), weights,
            };
            meshInfo[`${slotName}/${attachmentName}`] = {
              weighted, slots: weighted ? influenceCount : uvs.length / 2,
            };
          } else if (type === "clipping") {
            converted = { type: "clipping", vertices: list(attachment.vertices).map(Number), end_slot: attachment.end ?? null };
          } else if (type === "boundingbox") {
            converted = { type: "boundingbox", vertices: list(attachment.vertices).map(Number), weights: [] };
          } else if (type === "point") {
            converted = { type: "point", x: num(attachment.x), y: num(attachment.y), rotation: num(attachment.rotation) };
          } else if (type === "path") {
            converted = { type: "path", vertices: list(attachment.vertices).map(Number),
              closed: Boolean(attachment.closed), constant_speed: attachment.constantSpeed !== false };
          } else {
            report.lossy.push({ what: "attachment", where: attachmentName, detail: `\`${type}\` attachments are not read` });
          }
          if (converted) result.entries.push({ slot: slotName, name: attachmentName, attachment: converted });
        }
      }
      project.skins.push(result);
    }
    if (project.skins.length === 0) project.skins.push({ name: "default", entries: [] });
    project.default_skin = project.skins[0].name;

    const pending = [];
    for (const constraint of list(doc.constraints)) pending.push(constraint);
    for (const constraint of list(doc.ik)) pending.push({ ...constraint, type: "ik" });
    for (const constraint of list(doc.transform)) pending.push({ ...constraint, type: "transform" });
    for (const constraint of list(doc.path)) pending.push({ ...constraint, type: "path" });
    for (const constraint of list(doc.physics)) pending.push({ ...constraint, type: "physics" });
    pending.sort((a, b) => num(a.order) - num(b.order));
    const constraintNames = new Set();
    for (const constraint of pending) {
      const type = constraint.type;
      const name = String(constraint.name || type || "constraint");
      if (type === "ik") {
        project.constraints.push({
          name, type: "ik", target: String(constraint.target || ""), bones: list(constraint.bones),
          bend_direction: constraint.bendPositive === false ? -1 : 1,
          mix: num(constraint.mix, 1), softness: num(constraint.softness),
          stretch: Boolean(constraint.stretch), stretch_limit: 1.1, stiffness: 0,
        });
      } else if (type === "transform") {
        const target = constraint.source || constraint.target || "";
        project.constraints.push({
          name, type: "transform", target: String(target), bones: list(constraint.bones),
          bend_direction: 1, mix: 1, softness: 0, stretch: false, stretch_limit: 1.1,
          transform_mix: {
            rotate: num(constraint.mixRotate), translate_x: num(constraint.mixX),
            translate_y: num(constraint.mixY), scale_x: num(constraint.mixScaleX),
            scale_y: num(constraint.mixScaleY), shear_x: num(constraint.mixShearX),
            shear_y: num(constraint.mixShearY),
          },
          offsets: [num(constraint.x), num(constraint.y), num(constraint.rotation),
            1 + num(constraint.scaleX), 1 + num(constraint.scaleY), 0, num(constraint.shearY)],
          local: Boolean(constraint.local), relative: Boolean(constraint.relative),
        });
      } else {
        report.lossy.push({ what: "constraint", where: name, detail: `\`${type}\` constraints are not read yet` });
        continue;
      }
      project.constraint_order.push(name);
      constraintNames.add(name);
    }

    for (const [animationName, animation] of Object.entries(object(doc.animations))) {
      const result = { name: animationName, duration: 0.1, looping: true, timelines: [], events: [] };
      const note = (time) => { result.duration = Math.max(result.duration, num(time)); };
      for (const [bone, tracks] of Object.entries(object(animation.bones))) {
        if (!boneNames.has(bone)) continue;
        for (const [kind, frames] of Object.entries(object(tracks))) {
          for (const frame of list(frames)) note(frame.time);
          const base = `${animationName}/${bone}/${kind}`;
          if (kind === "rotate") {
            result.timelines.push({ kind: "bone_rotate", bone,
              keys: scalarKeys(frames, 0, (key) => num(key.value), report, base) });
          } else if (["translate", "scale", "shear"].includes(kind)) {
            for (const [axis, channel] of [["x", 0], ["y", 1]]) {
              const fallback = kind === "scale" ? 1 : 0;
              result.timelines.push({ kind: `bone_${kind}`, bone, axis,
                keys: scalarKeys(frames, channel, (key) => num(key[axis], fallback), report, `${base}/${axis}`) });
            }
          }
        }
      }
      for (const [slot, tracks] of Object.entries(object(animation.slots))) {
        if (!slotNames.has(slot)) continue;
        for (const [kind, frames] of Object.entries(object(tracks))) {
          for (const frame of list(frames)) note(frame.time);
          if (kind === "attachment") {
            result.timelines.push({ kind: "slot_attachment", slot,
              keys: list(frames).map((key) => ({ time: num(key.time), value: key.name ?? null })) });
          } else if (kind === "rgba" || kind === "rgb") {
            result.timelines.push({ kind: "slot_color", slot,
              keys: colorKeys(frames, report, `${animationName}/${slot}/${kind}`) });
          }
        }
      }
      for (const [constraint, frames] of Object.entries(object(animation.ik))) {
        if (!constraintNames.has(constraint)) continue;
        for (const frame of list(frames)) note(frame.time);
        result.timelines.push({ kind: "ik_mix", constraint,
          keys: scalarKeys(frames, 0, (key) => num(key.mix, 1), report, `${animationName}/${constraint}/ik`) });
        if (list(frames).some((key) => key.softness !== undefined)) {
          result.timelines.push({ kind: "ik_softness", constraint,
            keys: scalarKeys(frames, 1, (key) => num(key.softness), report, `${animationName}/${constraint}/ik`) });
        }
        const bends = list(frames).map((key) => key.bendPositive === false ? -1 : 1);
        if (bends.some((value, index) => index && value !== bends[index - 1])) {
          result.timelines.push({ kind: "ik_bend_direction", constraint,
            keys: list(frames).map((key) => ({ time: num(key.time), value: key.bendPositive === false ? -1 : 1, curve: "stepped" })) });
        }
      }
      for (const [constraint, frames] of Object.entries(object(animation.transform))) {
        if (!constraintNames.has(constraint)) continue;
        for (const frame of list(frames)) note(frame.time);
        result.timelines.push({ kind: "transform_constraint_mix", constraint,
          keys: list(frames).map((key, index, all) => ({
            time: num(key.time), rotate: num(key.mixRotate), translate_x: num(key.mixX),
            translate_y: num(key.mixY), scale_x: num(key.mixScaleX), scale_y: num(key.mixScaleY),
            shear_x: num(key.mixShearX), shear_y: num(key.mixShearY),
            ...(index === 0 ? linear() : interp(all[index - 1].curve, num(all[index - 1].time),
              num(all[index - 1].mixRotate), num(key.time), num(key.mixRotate), 0, report,
              `${animationName}/${constraint}/transform`)),
          })) });
      }
      for (const slotsOfSkin of Object.values(object(animation.attachments))) {
        for (const [slot, attachments] of Object.entries(object(slotsOfSkin))) {
          for (const [attachment, tracks] of Object.entries(object(attachments))) {
            const frames = list(tracks.deform);
            const info = meshInfo[`${slot}/${attachment}`];
            if (!info || frames.length === 0) continue;
            for (const frame of frames) note(frame.time);
            result.timelines.push({ kind: "deform", slot, attachment,
              keys: frames.map((frame, index, all) => {
                const offsets = Array(info.slots * 2).fill(0);
                const start = Math.max(0, Math.trunc(num(frame.offset)));
                for (let i = 0; i < list(frame.vertices).length && start + i < offsets.length; i++) {
                  offsets[start + i] = num(frame.vertices[i]);
                }
                return { time: num(frame.time), offsets,
                  ...(index === 0 ? linear() : interp(all[index - 1].curve,
                    num(all[index - 1].time), 0, num(frame.time), 0, 0, report,
                    `${animationName}/${slot}/${attachment}/deform`)) };
              }) });
          }
        }
      }
      for (const event of list(animation.events)) {
        note(event.time);
        result.events.push({ time: num(event.time), name: String(event.name || "event"),
          int_value: Math.trunc(num(event.int)), float_value: num(event.float),
          string_value: String(event.string || ""), audio: "", volume: 1, balance: 0 });
      }
      project.animations.push(result);
    }

    ankhimate.importProject(project, images, report);
  }

  ankhimate.registerImporter({
    id: "import.spine",
    label: "Spine JSON (community)",
    extensions: ["json"],
    canRead(text) {
      try {
        const value = JSON.parse(text);
        return Array.isArray(value.bones) && value.bones.length > 0 && Boolean(value.skeleton);
      } catch (_) { return false; }
    },
    read: convert,
  });

  ankhimate.registerExporter({
    id: "export.spine",
    label: "Spine JSON (community)",
    write() {
      const preset = ankhimate.resource("spine_json.json");
      if (!preset) throw new Error("spine_json.json is missing from the plugin package");
      emitPreset(JSON.parse(preset));
    },
  });
})();
