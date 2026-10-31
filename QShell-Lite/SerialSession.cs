// 串口会话：用 System.IO.Ports.SerialPort 打开 COM 口，收到的字节喂给终端。
// 参数来自 HostEntry：host = COM 名，port = 波特率（和桌面版一致），
// 其余走 serial_* 字段。

using System;
using System.Drawing;
using System.IO.Ports;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class SerialSession : ISession
    {
        public HostEntry Entry { get; private set; }

        private readonly VtTerminal term;
        private readonly TerminalView view;
        private readonly Timer pumpTimer = new Timer();

        private SerialPort port;
        private bool disposed;
        private volatile bool dirty;

        public event Action<string> Message;

        public TabPage Page { get; set; }

        // 串口没有这些能力，留空实现让宿主可以统一处理
        public bool AutoReconnect { get; set; }
        public bool LogEnabled { get; set; }

        public TerminalView View
        {
            get { return view; }
        }

        public bool IsConnected
        {
            get
            {
                try
                {
                    return port != null && port.IsOpen;
                }
                catch
                {
                    return false;
                }
            }
        }

        public SerialSession(HostEntry entry, int cols, int rows)
        {
            Entry = entry;
            term = new VtTerminal(rows, cols);
            view = new TerminalView(term);
            view.Input = Send;
            view.SizeChangedHandlers = OnViewResized;

            pumpTimer.Interval = 25;
            pumpTimer.Tick += OnPump;
        }

        /// <summary>当前系统上的串口列表（COM1、COM3…）。</summary>
        public static string[] ListPorts()
        {
            try
            {
                return SerialPort.GetPortNames();
            }
            catch
            {
                return new string[0];
            }
        }

        public bool Connect(out string error)
        {
            error = "";
            try
            {
                int baud = Entry.Port > 0 ? Entry.Port : 9600;
                port = new SerialPort(Entry.Host, baud);

                switch (Entry.SerialDataBits)
                {
                    case 5: port.DataBits = 5; break;
                    case 6: port.DataBits = 6; break;
                    case 7: port.DataBits = 7; break;
                    default: port.DataBits = 8; break;
                }

                if (Entry.SerialParity == "odd") port.Parity = Parity.Odd;
                else if (Entry.SerialParity == "even") port.Parity = Parity.Even;
                else port.Parity = Parity.None;

                port.StopBits = Entry.SerialStopBits == 2 ? StopBits.Two : StopBits.One;

                if (Entry.SerialFlowControl == "hardware") port.Handshake = Handshake.RequestToSend;
                else if (Entry.SerialFlowControl == "software") port.Handshake = Handshake.XOnXOff;
                else port.Handshake = Handshake.None;

                port.ReadTimeout = 1000;
                port.WriteTimeout = 1000;
                port.Open();

                port.DtrEnable = Entry.SerialDtr;
                port.RtsEnable = Entry.SerialRts;

                pumpTimer.Start();
                if (Message != null) Message("串口已打开: " + Entry.Host + " @" + baud);
                return true;
            }
            catch (Exception ex)
            {
                error = ex.Message;
                Close();
                return false;
            }
        }

        public void Close()
        {
            pumpTimer.Stop();
            try
            {
                if (port != null && port.IsOpen) port.Close();
            }
            catch { }
            try
            {
                if (port != null) port.Dispose();
            }
            catch { }
            port = null;
        }

        public void SendText(string text)
        {
            if (string.IsNullOrEmpty(text)) return;
            Send(System.Text.Encoding.UTF8.GetBytes(text));
        }

        public void Dispose()
        {
            if (disposed) return;
            disposed = true;
            Close();
        }

        private void OnPump(object sender, EventArgs e)
        {
            if (disposed) return;
            SerialPort p = port;
            if (p == null) return;

            try
            {
                while (p.BytesToRead > 0)
                {
                    int n = Math.Min(p.BytesToRead, 8192);
                    var buf = new byte[n];
                    int got = p.Read(buf, 0, n);
                    if (got <= 0) break;

                    if (Entry.SerialLocalEcho)
                    {
                        // 本地回显：把自己的输入也显示出来
                    }
                    term.Feed(buf, 0, got);
                }
            }
            catch (TimeoutException)
            {
            }
            catch
            {
                if (Message != null) Message("串口读取失败，连接可能已断开");
                Close();
            }
        }

        private void Send(byte[] data)
        {
            SerialPort p = port;
            if (p == null || !p.IsOpen) return;
            try
            {
                p.Write(data, 0, data.Length);
            }
            catch
            {
            }
        }

        private void OnViewResized()
        {
            int cols = Math.Max(10, view.ViewWidth / view.CellWidth);
            int rows = Math.Max(3, view.Height / view.CellHeight);
            if (cols != term.Cols || rows != term.Rows)
            {
                term.Resize(rows, cols);
            }
        }
    }
}
