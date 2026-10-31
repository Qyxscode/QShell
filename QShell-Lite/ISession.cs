// 会话接口：SSH 和串口都实现它，宿主只认这个。

using System;
using System.Windows.Forms;

namespace QShellLite
{
    internal interface ISession : IDisposable
    {
        HostEntry Entry { get; }
        TerminalView View { get; }
        bool IsConnected { get; }

        /// <summary>由宿主设置的标签页。</summary>
        TabPage Page { get; set; }

        /// <summary>状态消息（连接/断开/重连）。</summary>
        event Action<string> Message;

        bool Connect(out string error);
        void Close();
        void SendText(string text);


        bool AutoReconnect { get; set; }
        bool LogEnabled { get; set; }

    }
}
