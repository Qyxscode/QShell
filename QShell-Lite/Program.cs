// 程序入口 + 版本信息。
// QShell Lite 有独立的版本节奏，不和桌面版（QShell-WinUI3）共用版本号。

using System;
using System.Reflection;
using System.Windows.Forms;

[assembly: AssemblyTitle("QShell Lite")]
[assembly: AssemblyDescription("老系统（XP / Win7 / 11）上的轻量 SSH 终端客户端，自写 VT 引擎")]
[assembly: AssemblyProduct("QShell Lite")]
[assembly: AssemblyCopyright("QShell")]
[assembly: AssemblyVersion("0.3.0.0")]
[assembly: AssemblyFileVersion("0.3.0.0")]

namespace QShellLite
{
    internal static class Program
    {
        [STAThread]
        private static void Main(string[] args)
        {
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);

            // 可选：--config <path> 指定配置文件，不传就用桌面版默认路径
            string configPath = null;
            for (int i = 0; i < args.Length; i++)
            {
                if (string.Equals(args[i], "--config", StringComparison.OrdinalIgnoreCase)
                    && i + 1 < args.Length)
                {
                    configPath = args[i + 1];
                    i++;
                }
            }

            Application.Run(new LiteForm(configPath));
        }
    }
}
