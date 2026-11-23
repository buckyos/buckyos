/** In the BuckyOS App the system browser is opened through the bridge; elsewhere in a new tab. */
export async function openExternalUrl(url: string): Promise<boolean> {
  const buckyApi = (
    window as Window & {
      BuckyApi?: { openExternalUrl?: (url: string) => Promise<{ code: number }> };
    }
  ).BuckyApi;
  if (!buckyApi) {
    window.open(url, "_blank", "noopener,noreferrer");
    return true;
  }
  try {
    return (await buckyApi.openExternalUrl?.(url))?.code === 0;
  } catch {
    return false;
  }
}
