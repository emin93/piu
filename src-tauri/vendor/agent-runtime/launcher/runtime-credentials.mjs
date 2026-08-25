import { KeychainCredentialStore } from "./credential-store.mjs";
import { createProviderLock } from "./provider-lock.mjs";
import { createSecurityKeychain } from "./security-keychain.mjs";

export function createRuntimeCredentials({ credentialLockDirectory, lock }) {
  return new KeychainCredentialStore({
    keychain: createSecurityKeychain(),
    providerCredentialTypes: { "openai-codex": "oauth" },
    withProviderLock: createProviderLock({ lock, lockRoot: credentialLockDirectory }),
  });
}
