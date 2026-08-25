function throwIfAborted(signal) {
  signal?.throwIfAborted();
}

function assertCredential(providerId, expectedType, credential) {
  if (credential === null || typeof credential !== "object" || credential.type !== expectedType) {
    throw new Error(`invalid credential for ${providerId}`);
  }
  if (
    expectedType === "oauth" &&
    (typeof credential.access !== "string" ||
      typeof credential.refresh !== "string" ||
      typeof credential.expires !== "number")
  ) {
    throw new Error(`invalid OAuth credential for ${providerId}`);
  }
}

export class KeychainCredentialStore {
  #keychain;
  #providerCredentialTypes;
  #withProviderLock;

  constructor({ keychain, providerCredentialTypes, withProviderLock }) {
    this.#keychain = keychain;
    this.#providerCredentialTypes = new Map(Object.entries(providerCredentialTypes));
    this.#withProviderLock = withProviderLock;
  }

  async read(providerId, options = {}) {
    throwIfAborted(options.signal);
    if (!this.#providerCredentialTypes.has(providerId)) return undefined;
    return this.#keychain.read(providerId, options);
  }

  async list(options = {}) {
    throwIfAborted(options.signal);
    const entries = [...this.#providerCredentialTypes.entries()];
    const present = await Promise.all(
      entries.map(async ([providerId, type]) => {
        throwIfAborted(options.signal);
        return (await this.#keychain.exists(providerId, options))
          ? { providerId, type }
          : undefined;
      }),
    );
    return present.filter((entry) => entry !== undefined);
  }

  async modify(providerId, update, options = {}) {
    throwIfAborted(options.signal);
    const expectedType = this.#providerCredentialTypes.get(providerId);
    if (!expectedType) throw new Error(`unsupported credential provider ${providerId}`);
    return this.#withProviderLock(providerId, options.signal, async () => {
      throwIfAborted(options.signal);
      const current = await this.#keychain.read(providerId, options);
      throwIfAborted(options.signal);
      const next = await update(current);
      throwIfAborted(options.signal);
      if (next === undefined) return current;
      assertCredential(providerId, expectedType, next);
      await this.#keychain.write(providerId, next, options);
      return next;
    });
  }

  async delete(providerId, options = {}) {
    throwIfAborted(options.signal);
    if (!this.#providerCredentialTypes.has(providerId)) return;
    await this.#withProviderLock(providerId, options.signal, async () => {
      throwIfAborted(options.signal);
      await this.#keychain.delete(providerId, options);
    });
  }
}
