export function startAmbientCanvas(canvas: HTMLCanvasElement): () => void {
  const context = canvas.getContext("2d", { alpha: false });
  if (!context) return () => undefined;

  let width = 0;
  let height = 0;
  let scale = 1;
  let frameId = 0;

  const resize = () => {
    scale = Math.min(window.devicePixelRatio || 1, 1.5);
    width = window.innerWidth;
    height = window.innerHeight;
    canvas.width = width * scale;
    canvas.height = height * scale;
    context.setTransform(scale, 0, 0, scale, 0, 0);
  };

  const ribbon = (time: number, offset: number, lineWidth: number, alpha: number) => {
    context.save();
    context.globalAlpha = alpha;
    context.filter = "blur(30px)";
    context.lineCap = "round";
    context.strokeStyle = "#d0e4e7";
    context.lineWidth = lineWidth;
    context.beginPath();
    for (let x = -180; x <= width + 180; x += 22) {
      const progress = x / width;
      const y = height * (0.18 + 0.27 * Math.sin(progress * 3.35 + offset + time * 0.00013)
        + 0.09 * Math.sin(progress * 7.7 - offset * 0.7 + time * 0.00006));
      if (x === -180) context.moveTo(x, y);
      else context.lineTo(x, y);
    }
    context.stroke();
    context.restore();
  };

  const draw = (time: number) => {
    const gradient = context.createLinearGradient(0, 0, width, height);
    gradient.addColorStop(0, "#051427");
    gradient.addColorStop(0.5, "#093969");
    gradient.addColorStop(1, "#2665a3");
    context.fillStyle = gradient;
    context.fillRect(0, 0, width, height);
    context.save();
    context.globalAlpha = 0.065;
    context.strokeStyle = "#76b9eb";
    context.lineWidth = 0.5;
    for (let x = -height; x < width + height; x += 78) {
      context.beginPath();
      context.moveTo(x, 0);
      context.lineTo(x - height * 0.35, height);
      context.stroke();
    }
    for (let y = 0; y < height; y += 78) {
      context.beginPath();
      context.moveTo(0, y);
      context.lineTo(width, y);
      context.stroke();
    }
    context.restore();
    ribbon(time, 0.6, 108, 0.31);
    ribbon(time, 3.05, 72, 0.16);
    frameId = requestAnimationFrame(draw);
  };

  window.addEventListener("resize", resize);
  resize();
  frameId = requestAnimationFrame(draw);
  return () => {
    window.removeEventListener("resize", resize);
    cancelAnimationFrame(frameId);
  };
}
