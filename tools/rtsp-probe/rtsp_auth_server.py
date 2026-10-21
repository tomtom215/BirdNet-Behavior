#!/usr/bin/env python3
"""A one-stream RTSP server that wants Basic credentials, for checking how ffmpeg opens one.

    RTSP_PROBE_PASSWORD=<password> python3 tools/rtsp-probe/rtsp_auth_server.py   # listens on 127.0.0.1:18554
    ffmpeg -rtsp_transport tcp -i rtsp://viewer:<password>@127.0.0.1:18554/live -t 2 -f s16le -ac 1 -ar 8000 out.raw

It answers `401` to DESCRIBE until the request carries `Authorization: Basic` for
`viewer` and the password in `RTSP_PROBE_PASSWORD`, then serves three seconds of G.711 silence as interleaved RTP
over TCP. Every request is logged to stderr with whether it carried the right credentials
(`auth=ok`), wrong ones (`auth=wrong`) or none, so a run shows whether they reached the camera.

It exists to check one thing, recorded in docs/TECH_DEBT.md (TD-5): that ffmpeg 6.1.1 can
open an RTSP stream whose address is in a `concat` list file, with the credentials in the
file and not on the command line. It is not a camera and is not used by any test.
"""
import asyncio, base64, os, struct, sys, time
USER = "viewer"
PASS = os.environ.get("RTSP_PROBE_PASSWORD", "")
if not PASS:
    sys.exit("set RTSP_PROBE_PASSWORD to the password the server should ask for")
SDP = ("v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=Test\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n"
       "m=audio 0 RTP/AVP 0\r\na=control:streamid=0\r\n")
async def handle(reader, writer):
    seq = 0; ts = 0; playing = False
    async def send(code, text, cseq, extra="", body=""):
        hdr = f"RTSP/1.0 {code} {text}\r\nCSeq: {cseq}\r\n{extra}"
        if body:
            hdr += f"Content-Type: application/sdp\r\nContent-Length: {len(body)}\r\n"
        writer.write((hdr + "\r\n" + body).encode()); await writer.drain()
    try:
        while True:
            head = await reader.readuntil(b"\r\n\r\n")
            lines = head.decode().split("\r\n")
            method = lines[0].split(" ")[0]
            h = {l.split(":",1)[0].lower(): l.split(":",1)[1].strip() for l in lines[1:] if ":" in l}
            cseq = h.get("cseq", "0")
            auth = h.get("authorization", "")
            ok = auth.startswith("Basic") and base64.b64decode(auth.split()[1]).decode() == f"{USER}:{PASS}"
            verdict = "ok" if ok else ("wrong" if auth else "none")
            print(f"{time.time():.2f} {method} auth={verdict}", file=sys.stderr, flush=True)
            if method != "OPTIONS":
                if not ok:
                    await send(401, "Unauthorized", cseq, 'WWW-Authenticate: Basic realm="cam"\r\n'); continue
            if method == "OPTIONS":
                await send(200, "OK", cseq, "Public: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN\r\n")
            elif method == "DESCRIBE":
                await send(200, "OK", cseq, "", SDP)
            elif method == "SETUP":
                await send(200, "OK", cseq, "Session: 12345678\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n")
            elif method == "PLAY":
                await send(200, "OK", cseq, "Session: 12345678\r\nRange: npt=0.000-\r\n")
                playing = True
                for i in range(150):  # 3 s of 20 ms packets
                    pkt = struct.pack(">BBHII", 0x80, 0, seq & 0xFFFF, ts & 0xFFFFFFFF, 0x1234) + b"\xff" * 160
                    writer.write(b"$" + struct.pack(">BH", 0, len(pkt)) + pkt); await writer.drain()
                    seq += 1; ts += 160
                    await asyncio.sleep(0.02)
                break
            elif method == "TEARDOWN":
                await send(200, "OK", cseq, "Session: 12345678\r\n"); break
            else:
                await send(501, "Not Implemented", cseq)
    except (asyncio.IncompleteReadError, ConnectionError):
        pass
    finally:
        writer.close()
async def main():
    srv = await asyncio.start_server(handle, "127.0.0.1", 18554)
    async with srv: await srv.serve_forever()
asyncio.run(main())
