// An explicitly supplied saved value. This module never creates an identity.
export class SavedDevice {
  #value; #seeded = false;
  constructor(input) {
    if (!input || Object.keys(input).join(',') !== 'id'
        || typeof input.id !== 'string' || !/^[A-Za-z0-9._~-]{1,256}$/.test(input.id))
      throw new Error('device');
    this.#value = input.id;
    Object.freeze(this);
  }
  seed(window) {
    if (this.#seeded) throw new Error('device-already-seeded');
    this.#seeded = true;
    const storage = window.localStorage;
    if (storage.length !== 0 || window.document.cookie !== '')
      throw new Error('device-storage-not-empty');
    storage.setItem('__cuid', this.#value);
    window.document.cookie = '__cuid=' + this.#value
      + '; Domain=.framer.com; Path=/; Secure; SameSite=None';
    const verify = () => {
      const values = window.document.cookie.split(';').map(part => part.trim())
        .filter(part => part.startsWith('__cuid='));
      if (storage.getItem('__cuid') !== this.#value || values.length !== 1
          || values[0] !== '__cuid=' + this.#value)
        throw new Error('device-storage-changed');
    };
    verify();
    return verify;
  }
  toJSON() { throw new Error('private-device'); }
}
