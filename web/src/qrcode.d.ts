declare module "qrcode" {
  interface QRCodeApi {
    toString(text: string, options?: { type?: "svg"; margin?: number }): Promise<string>;
  }
  const qrcode: QRCodeApi;
  export default qrcode;
}
