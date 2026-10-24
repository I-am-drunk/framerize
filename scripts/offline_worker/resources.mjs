// Local PNGs only. Never delegate to the network or filesystem loader.
export function pngResources(ResourceLoader) {
  return class extends ResourceLoader {
    #images = 0;
    constructor(options = { userAgent: 'Framerize/0.1' }) { super(options); }
    fetch(url) {
      if (typeof url !== 'string' || url.length > 87406) return null;
      const match = /^data:image\/png;base64,([A-Za-z0-9+/]*={0,2})$/.exec(url);
      if (!match || match[1].length > 87384) return null;
      const bytes = Buffer.from(match[1], 'base64');
      if (bytes.length > 65536 || bytes.toString('base64') !== match[1]) return null;
      if (bytes.length < 33 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a'
          || bytes.readUInt32BE(8) !== 13 || bytes.toString('ascii', 12, 16) !== 'IHDR')
        return null;
      const width = bytes.readUInt32BE(16), height = bytes.readUInt32BE(20);
      if (!width || !height || width > 2048 || height > 2048
          || width * height > 262144 || this.#images >= 4) return null;
      this.#images++;
      const result = Promise.resolve(bytes);
      result.abort = () => {};
      return result;
    }
  };
}
