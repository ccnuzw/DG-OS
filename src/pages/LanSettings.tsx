// V1-FR-007 §2.3 item 6 (D085): the LAN collaboration entry before FR-010 exists. It states that the capability is
// not enabled and calls no host command: there is no LAN service, so there is no state to show. The switch, the
// service status, the address, devices and credentials all arrive with V1-FR-010 (D039).
export default function LanSettings() {
  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-6 p-6 text-foreground">
      <h1 className="m-0 text-2xl font-semibold text-foreground">局域网协同</h1>
      <div role="status" className="rounded-lg border border-info bg-info-bg p-4 text-sm text-info-text wrap-anywhere">
        <p className="m-0 font-semibold">局域网协同尚未启用</p>
        <p className="m-0 mt-2">
          本版本不运行局域网服务：不监听端口，不接受其他设备的连接，也不生成访问凭据。
        </p>
      </div>
      <p className="m-0 text-sm text-muted-foreground">
        服务开关、运行状态与访问地址、已连接设备和访问凭据将随“局域网协同”功能（V1-FR-010）提供。
      </p>
    </div>
  );
}
