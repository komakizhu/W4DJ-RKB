import { beforeEach, describe, expect, it, vi } from 'vitest';

const { toDataURL } = vi.hoisted(() => ({ toDataURL: vi.fn() }));

vi.mock('qrcode', () => ({
  default: { toDataURL },
}));

import { renderPlaintextQrDataUrl } from './qr-code';

describe('renderPlaintextQrDataUrl', () => {
  beforeEach(() => {
    toDataURL.mockReset();
  });

  it('passes the exact plaintext and fixed local QR options', async () => {
    toDataURL.mockResolvedValue('data:image/png;base64,abc');

    await expect(renderPlaintextQrDataUrl('歌名 - 歌手')).resolves.toBe('data:image/png;base64,abc');
    expect(toDataURL).toHaveBeenCalledWith('歌名 - 歌手', {
      errorCorrectionLevel: 'M',
      margin: 2,
      width: 512,
      type: 'image/png',
    });
  });

  it('rejects an empty payload before invoking the renderer', async () => {
    await expect(renderPlaintextQrDataUrl('')).rejects.toThrow('二维码内容不能为空');
    expect(toDataURL).not.toHaveBeenCalled();
  });

  it('surfaces renderer failures', async () => {
    toDataURL.mockRejectedValue(new Error('canvas unavailable'));

    await expect(renderPlaintextQrDataUrl('track')).rejects.toThrow('canvas unavailable');
  });
});
