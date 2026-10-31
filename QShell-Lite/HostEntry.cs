// 主机数据 + 配置读取（对应桌面版 %APPDATA%\QShell\config.json 的 devices[].hosts[]）
// 拆分自原 QShellLite.cs，逻辑未改动。

using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Web.Script.Serialization;

namespace QShellLite
{
    /// <summary>一条可连接的主机记录。</summary>
    internal sealed class HostEntry
    {
        public string Group = "";
        public string Name = "";
        public string Protocol = "";
        public string Host = "";
        public int Port = 22;
        public string User = "";
        public string Pwd = "";
        public string KeyPath = "";
        public string Memo = "";

        // 串口参数（protocol == "serial" 时有效；host 存 COM 名，port 存波特率）
        public int SerialDataBits = 8;
        public string SerialParity = "none";
        public int SerialStopBits = 1;
        public string SerialFlowControl = "none";
        public string SerialNewline = "";
        public bool SerialLocalEcho;
        public bool SerialDtr = true;
        public bool SerialRts = true;

        // 跳板机（ProxyJump）：留空表示直连
        public string ProxyHost = "";
        public int ProxyPort = 22;
        public string ProxyUser = "";
        public string ProxyPwd = "";
        public string ProxyKey = "";

        /// <summary>最近一次探测的延迟（毫秒），-1 表示未测。</summary>
        public int LatencyMs = -1;

        /// <summary>在 config.json 里的位置（-1 表示不在文件里，比如示例数据）。</summary>
        public int DeviceIndex = -1;
        public int HostIndex = -1;

        /// <summary>原始的 JSON 节点。改字段时直接改它，能把桌面版的未知字段原样保留。</summary>
        internal Dictionary<string, object> Raw;

        /// <summary>给人看的名字：没有 name 就退回 host。</summary>
        public string DisplayName
        {
            get { return string.IsNullOrEmpty(Name) ? Host : Name; }
        }

        /// <summary>命令行里的 user@host 部分。</summary>
        public string Target
        {
            get { return string.IsNullOrEmpty(User) ? Host : User + "@" + Host; }
        }

        /// <summary>同一个目标（重新载入配置后对象会换，所以按字段比）。</summary>
        public bool SameTarget(HostEntry other)
        {
            if (other == null)
            {
                return false;
            }
            return Host == other.Host
                && Port == other.Port
                && User == other.User
                && Protocol == other.Protocol;
        }

        public override string ToString()
        {
            return string.IsNullOrEmpty(Group) ? DisplayName : Group + " / " + DisplayName;
        }
    }

    /// <summary>读桌面版的 config.json；读不出来就退到示例数据。</summary>
    internal static class ConfigStore
    {
        /// <summary>桌面版默认配置路径。</summary>
        public static string DefaultPath
        {
            get
            {
                return Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                    Path.Combine("QShell", "config.json"));
            }
        }

        public static List<HostEntry> Load(string path)
        {
            string json = File.ReadAllText(path, Encoding.UTF8);
            return Parse(json);
        }

        /// <summary>
        /// 读取整个 config.json 的根节点。主机管理直接改这个字典里的对应节点再写回，
        /// 这样桌面版新增的字段（details、settings 等）都会原样保留。
        /// </summary>
        public static Dictionary<string, object> ReadRoot(string path)
        {
            string json = File.ReadAllText(path, Encoding.UTF8);
            var serializer = new JavaScriptSerializer();
            serializer.MaxJsonLength = int.MaxValue;
            return serializer.Deserialize<Dictionary<string, object>>(json);
        }

        /// <summary>写回 config.json（先备份成 .bak）。</summary>
        public static void WriteRoot(string path, Dictionary<string, object> root)
        {
            var serializer = new JavaScriptSerializer();
            serializer.MaxJsonLength = int.MaxValue;
            string json = serializer.Serialize(root);

            if (File.Exists(path))
            {
                File.Copy(path, path + ".bak", true);
            }
            File.WriteAllText(path, json, new UTF8Encoding(false));
        }

        /// <summary>没配置文件时放几条示例，方便先看界面与交互。</summary>
        public static List<HostEntry> Samples()
        {
            var result = new List<HostEntry>();

            var first = new HostEntry();
            first.Group = "示例分组";
            first.Name = "演示主机 1";
            first.Protocol = "ssh";
            first.Host = "192.168.1.10";
            first.Port = 22;
            first.User = "root";
            result.Add(first);

            var second = new HostEntry();
            second.Group = "示例分组";
            second.Name = "演示主机 2";
            second.Protocol = "ssh";
            second.Host = "192.168.1.11";
            second.Port = 22;
            second.User = "admin";
            result.Add(second);

            var third = new HostEntry();
            third.Group = "示例分组";
            third.Name = "OpenWrt 路由";
            third.Protocol = "ssh";
            third.Host = "192.168.1.1";
            third.Port = 22;
            third.User = "root";
            result.Add(third);

            return result;
        }

        private static List<HostEntry> Parse(string json)
        {
            var result = new List<HostEntry>();
            var serializer = new JavaScriptSerializer();
            serializer.MaxJsonLength = int.MaxValue;

            var root = serializer.Deserialize<Dictionary<string, object>>(json);
            if (root == null)
            {
                return result;
            }

            object devicesObj;
            if (!root.TryGetValue("devices", out devicesObj))
            {
                return result;
            }

            // 注意：用 JavaScriptSerializer.Deserialize<Dictionary<string, object>> 时，
            // 嵌套的 JSON 数组会被转成 ArrayList 而不是 object[]（实测 .NET 4.0 行为）。
            // 所以这里必须按 IList 取（object[] 与 ArrayList 都实现 IList）。
            var devices = devicesObj as IList;
            if (devices == null)
            {
                return result;
            }

            for (int di = 0; di < devices.Count; di++)
            {
                var device = devices[di] as Dictionary<string, object>;
                if (device == null)
                {
                    continue;
                }

                string group = AsString(device, "group");
                object hostsObj;
                if (!device.TryGetValue("hosts", out hostsObj))
                {
                    continue;
                }

                var hosts = hostsObj as IList;
                if (hosts == null)
                {
                    continue;
                }

                for (int hi = 0; hi < hosts.Count; hi++)
                {
                    var h = hosts[hi] as Dictionary<string, object>;
                    if (h == null)
                    {
                        continue;
                    }

                    var entry = new HostEntry();
                    entry.Group = group;
                    entry.Name = AsString(h, "name");
                    entry.Protocol = AsString(h, "protocol");
                    entry.Host = AsString(h, "host");
                    entry.Port = AsInt(h, "port", 22);
                    entry.User = AsString(h, "user");
                    entry.Pwd = AsString(h, "pwd");
                    entry.KeyPath = AsString(h, "key_path");
                    entry.Memo = ReadDetail(h, "memo");
                    entry.SerialDataBits = AsInt(h, "serial_data_bits", 8);
                    entry.SerialParity = AsString(h, "serial_parity");
                    if (entry.SerialParity.Length == 0) entry.SerialParity = "none";
                    entry.SerialStopBits = AsInt(h, "serial_stop_bits", 1);
                    entry.SerialFlowControl = AsString(h, "serial_flow_control");
                    if (entry.SerialFlowControl.Length == 0) entry.SerialFlowControl = "none";
                    entry.SerialNewline = AsString(h, "serial_newline");
                    entry.SerialLocalEcho = AsBool(h, "serial_local_echo", false);
                    entry.SerialDtr = AsBool(h, "serial_dtr", true);
                    entry.SerialRts = AsBool(h, "serial_rts", true);
                    entry.ProxyHost = AsString(h, "proxy_host");
                    entry.ProxyPort = AsInt(h, "proxy_port", 22);
                    entry.ProxyUser = AsString(h, "proxy_user");
                    entry.ProxyPwd = AsString(h, "proxy_pwd");
                    entry.ProxyKey = AsString(h, "proxy_key");
                    entry.DeviceIndex = di;
                    entry.HostIndex = hi;
                    entry.Raw = h;
                    result.Add(entry);
                }
            }

            return result;
        }

        /// <summary>读取 details 子对象里的字段（桌面版的备注、到期时间等都在这）。</summary>
        private static string ReadDetail(Dictionary<string, object> host, string key)
        {
            object detailsObj;
            if (!host.TryGetValue("details", out detailsObj)) return "";
            var details = detailsObj as Dictionary<string, object>;
            return details == null ? "" : AsString(details, key);
        }

        /// <summary>读取所有分组名（包括 hosts 为空的组）。</summary>
        public static List<string> LoadGroups(string path)
        {
            var result = new List<string>();
            try
            {
                Dictionary<string, object> root = ReadRoot(path);
                var devices = root == null ? null : root["devices"] as System.Collections.IList;
                if (devices == null) return result;

                foreach (object item in devices)
                {
                    var dev = item as Dictionary<string, object>;
                    if (dev == null) continue;
                    string group = Convert.ToString(dev["group"]);
                    if (group == null) group = "";
                    if (!result.Contains(group)) result.Add(group);
                }
            }
            catch
            {
            }
            return result;
        }

        /// <summary>读取 settings（不存在时返回空字典）。</summary>
        public static Dictionary<string, object> LoadSettings(Dictionary<string, object> root)
        {
            object obj;
            if (root != null && root.TryGetValue("settings", out obj))
            {
                var map = obj as Dictionary<string, object>;
                if (map != null) return map;
            }
            return new Dictionary<string, object>();
        }

        /// <summary>写回 settings。</summary>
        public static void SaveSettings(Dictionary<string, object> root, Dictionary<string, object> settings)
        {
            root["settings"] = settings;
        }

        /// <summary>读取 config.json 的 memos 命令备忘录。</summary>
        public static List<MemoGroup> LoadMemos(Dictionary<string, object> root)
        {
            var result = new List<MemoGroup>();
            object memosObj;
            if (!root.TryGetValue("memos", out memosObj)) return result;
            var memos = memosObj as System.Collections.IList;
            if (memos == null) return result;

            foreach (object mo in memos)
            {
                var m = mo as Dictionary<string, object>;
                if (m == null) continue;
                var g = new MemoGroup();
                g.Group = AsString(m, "group");

                object itemsObj;
                var items = m.TryGetValue("items", out itemsObj)
                    ? itemsObj as System.Collections.IList
                    : null;
                if (items != null)
                {
                    foreach (object io in items)
                    {
                        var it = io as Dictionary<string, object>;
                        if (it == null) continue;
                        var item = new MemoItem();
                        item.Name = AsString(it, "name");
                        item.Cmd = AsString(it, "cmd");
                        g.Items.Add(item);
                    }
                }
                result.Add(g);
            }
            return result;
        }

        /// <summary>把备忘录写回根节点（不动其他字段）。</summary>
        public static void SaveMemos(Dictionary<string, object> root, List<MemoGroup> groups)
        {
            var list = new System.Collections.ArrayList();
            foreach (MemoGroup g in groups)
            {
                var m = new Dictionary<string, object>();
                m["group"] = g.Group;
                var items = new System.Collections.ArrayList();
                foreach (MemoItem it in g.Items)
                {
                    var d = new Dictionary<string, object>();
                    d["name"] = it.Name;
                    d["cmd"] = it.Cmd;
                    items.Add(d);
                }
                m["items"] = items;
                list.Add(m);
            }
            root["memos"] = list;
        }

        private static string AsString(Dictionary<string, object> map, string key)
        {
            object value;
            if (map.TryGetValue(key, out value) && value != null)
            {
                return Convert.ToString(value);
            }
            return "";
        }

        private static bool AsBool(Dictionary<string, object> map, string key, bool fallback)
        {
            object v;
            if (map.TryGetValue(key, out v) && v != null)
            {
                if (v is bool) return (bool)v;
                string text = Convert.ToString(v).ToLowerInvariant();
                if (text == "true") return true;
                if (text == "false") return false;
            }
            return fallback;
        }

        private static int AsInt(Dictionary<string, object> map, string key, int fallback)
        {
            object value;
            if (map.TryGetValue(key, out value) && value != null)
            {
                int parsed;
                if (int.TryParse(Convert.ToString(value), out parsed))
                {
                    return parsed;
                }
            }
            return fallback;
        }
    }
}
