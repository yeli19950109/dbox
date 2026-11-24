import { createStore, del, get, set } from "idb-keyval";

// Keep Dev HTTP credentials in this browser's origin. Only the native app uses Keychain.
const store = createStore("dbox-dev-credentials", "credentials");
function entryForProfile(profileId: string | null): string {
  if (profileId == null || profileId === "default") return "llama-server";
  if (!/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(profileId)) {
    throw new Error("无效的配置 ID");
  }
  return `llama-server/${profileId.toLowerCase()}`;
}

interface EncryptedKey {
  key: CryptoKey;
  iv: Uint8Array<ArrayBuffer>;
  ciphertext: ArrayBuffer;
}

export async function readBrowserLlamaApiKey(profileId: string | null = null): Promise<string | null> {
  const saved = await get<EncryptedKey>(entryForProfile(profileId), store);
  if (!saved) return null;
  const data = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: saved.iv }, saved.key, saved.ciphertext,
  );
  return new TextDecoder().decode(data);
}

export async function saveBrowserLlamaApiKey(value: string, profileId: string | null = null): Promise<null> {
  const entry = entryForProfile(profileId);
  if (value === "") {
    await del(entry, store);
  } else {
    const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, new TextEncoder().encode(value));
    await set(entry, { key, iv, ciphertext } satisfies EncryptedKey, store);
  }
  return null;
}
