import QRCode from 'qrcode';

export type PlaintextQrOptions = {
  errorCorrectionLevel: 'M';
  margin: 2;
  width: 512;
  type: 'image/png';
};

export const PLAINTEXT_QR_OPTIONS: PlaintextQrOptions = {
  errorCorrectionLevel: 'M',
  margin: 2,
  width: 512,
  type: 'image/png',
};

/** Render a QR payload locally. The payload is deliberately plain text: no URL,
 * server, page metadata, or filesystem path is added here. */
export async function renderPlaintextQrDataUrl(text: string): Promise<string> {
  if (text.length === 0) {
    throw new Error('二维码内容不能为空');
  }
  return QRCode.toDataURL(text, PLAINTEXT_QR_OPTIONS);
}
