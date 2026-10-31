// 右侧备忘录面板：命令快捷方式，数据存 config.json 的 "memos"（和桌面版同一份）。
// 右键菜单对齐桌面版：条目上「插入到终端/立即执行/编辑/复制命令/删除」，
// 分组上「在此组新建命令/重命名分组/删除分组」，空白处「新建命令/新建命令分组」。

using System;
using System.Collections.Generic;
using System.Drawing;
using System.Windows.Forms;

namespace QShellLite
{
    internal sealed class MemoItem
    {
        public string Name = "";
        public string Cmd = "";
    }

    internal sealed class MemoGroup
    {
        public string Group = "";
        public List<MemoItem> Items = new List<MemoItem>();
    }

    internal sealed class MemoPanel : Panel
    {
        private readonly TreeView tree = new TreeView();
        private readonly Button addButton = new Button();
        private readonly Label titleLabel = new Label();
        private readonly ContextMenuStrip menu = new ContextMenuStrip();
        private TreeNode contextNode;

        private List<MemoGroup> groups = new List<MemoGroup>();

        /// <summary>执行命令：cmd, 是否附带回车执行。</summary>
        public event Action<string, bool> RunCommand;

        /// <summary>数据变更后通知宿主保存。</summary>
        public event Action Changed;

        public List<MemoGroup> CurrentGroups
        {
            get { return groups; }
        }

        public MemoPanel()
        {
            BackColor = SystemColors.Control;

            titleLabel.Text = "备忘录";
            titleLabel.Font = new Font(Font.FontFamily, 9f, FontStyle.Bold);
            titleLabel.AutoSize = true;
            titleLabel.Location = new Point(8, 8);
            Controls.Add(titleLabel);

            tree.BorderStyle = BorderStyle.FixedSingle;
            tree.HideSelection = false;
            tree.FullRowSelect = true;
            tree.ShowLines = true;
            tree.ShowPlusMinus = true;
            tree.ShowRootLines = true;
            tree.NodeMouseDoubleClick += OnNodeDoubleClick;
            tree.KeyDown += OnTreeKeyDown;
            tree.MouseDown += delegate(object s, MouseEventArgs e)
            {
                if (e.Button == MouseButtons.Right)
                {
                    TreeNode node = tree.GetNodeAt(e.X, e.Y);
                    tree.SelectedNode = node;   // 空白处点击时置空
                    contextNode = node;
                    RebuildMenu(node);
                }
            };
            tree.AllowDrop = true;
            tree.ItemDrag += delegate(object s, ItemDragEventArgs e)
            {
                tree.DoDragDrop(e.Item, DragDropEffects.Move);
            };
            tree.DragOver += delegate(object s, DragEventArgs e)
            {
                e.Effect = DragDropEffects.Move;
            };
            tree.DragDrop += OnTreeDragDrop;
            Controls.Add(tree);

            RebuildMenu(null);
            tree.ContextMenuStrip = menu;

            addButton.Text = "+ 添加命令项";
            addButton.Click += delegate { OnAddItem(null, null); };
            Controls.Add(addButton);

            Resize += delegate { ApplyLayout(); };
        }

        /// <summary>
        /// 按右键目标重建菜单。不用 Visible 切换：WinForms 在有隐藏项的
        /// ContextMenuStrip 上会留下重绘残影（文字叠在一起）。
        /// </summary>
        private void RebuildMenu(TreeNode node)
        {
            menu.Items.Clear();
            bool isItem = node != null && node.Tag is MemoItem;
            bool isGroup = node != null && node.Tag is MemoGroup;

            if (isItem)
            {
                menu.Items.Add("插入到终端", null, delegate { RunSelected(false); });
                menu.Items.Add("立即执行此命令", null, delegate { RunSelected(true); });
                menu.Items.Add("编辑此命令...", null, OnEditItem);
                menu.Items.Add("复制命令文本", null, OnCopyCommand);
                menu.Items.Add(new ToolStripSeparator());
                menu.Items.Add("删除命令", null, OnDeleteItem);
            }
            else if (isGroup)
            {
                menu.Items.Add("在此组新建命令...", null, OnAddItem);
                menu.Items.Add("重命名分组...", null, OnRenameGroup);
                menu.Items.Add("删除分组", null, OnDeleteGroup);
                menu.Items.Add(new ToolStripSeparator());
                menu.Items.Add("新建命令分组...", null, OnNewGroup);
            }
            else
            {
                menu.Items.Add("新建命令...", null, OnAddItem);
                menu.Items.Add("新建命令分组...", null, OnNewGroup);
            }
        }
        /// <summary>拖拽移动：条目可换组/组内排序，分组可整体排序。</summary>
        private void OnTreeDragDrop(object sender, DragEventArgs e)
        {
            var source = e.Data.GetData(typeof(TreeNode)) as TreeNode;
            if (source == null) return;

            Point pt = tree.PointToClient(new Point(e.X, e.Y));
            TreeNode target = tree.GetNodeAt(pt);

            if (source.Tag is MemoGroup)
            {
                MemoGroup moving = (MemoGroup)source.Tag;
                if (target == null || !(target.Tag is MemoGroup)) return;
                MemoGroup anchorGroup = (MemoGroup)target.Tag;
                if (ReferenceEquals(moving, anchorGroup)) return;

                groups.Remove(moving);
                int idx = groups.IndexOf(anchorGroup);
                Rectangle r = target.Bounds;
                if (pt.Y > r.Y + r.Height / 2) idx++;
                if (idx < 0) idx = 0;
                if (idx > groups.Count) idx = groups.Count;
                groups.Insert(idx, moving);
            }
            else if (source.Tag is MemoItem)
            {
                MemoItem item = (MemoItem)source.Tag;
                MemoGroup oldGroup = GroupOf(source);
                if (oldGroup == null) return;

                MemoGroup newGroup = null;
                int insertAt = 0;

                if (target == null)
                {
                    if (groups.Count == 0) return;
                    newGroup = groups[groups.Count - 1];
                    insertAt = newGroup.Items.Count;
                }
                else if (target.Tag is MemoGroup)
                {
                    newGroup = (MemoGroup)target.Tag;
                    insertAt = newGroup.Items.Count;
                }
                else if (target.Tag is MemoItem)
                {
                    newGroup = GroupOf(target);
                    if (newGroup == null) return;
                    insertAt = newGroup.Items.IndexOf((MemoItem)target.Tag);
                    if (insertAt < 0) insertAt = newGroup.Items.Count;
                    Rectangle r = target.Bounds;
                    if (pt.Y > r.Y + r.Height / 2) insertAt++;
                }
                else
                {
                    return;
                }

                oldGroup.Items.Remove(item);
                if (insertAt > newGroup.Items.Count) insertAt = newGroup.Items.Count;
                if (insertAt < 0) insertAt = 0;
                newGroup.Items.Insert(insertAt, item);
            }
            else
            {
                return;
            }

            SetGroups(groups);
            NotifyChanged();
        }

        private void ApplyLayout()
        {
            int w = ClientSize.Width;
            int h = ClientSize.Height;
            tree.Bounds = new Rectangle(6, 30, Math.Max(40, w - 12), Math.Max(40, h - 74));
            addButton.Bounds = new Rectangle(6, h - 38, Math.Max(40, w - 12), 30);
        }

        // ---------------- 数据 ----------------

        public void SetGroups(List<MemoGroup> data)
        {
            groups = data ?? new List<MemoGroup>();
            tree.BeginUpdate();
            try
            {
                tree.Nodes.Clear();
                foreach (MemoGroup g in groups)
                {
                    TreeNode gn = tree.Nodes.Add(g.Group.Length > 0 ? g.Group : "(未分组)");
                    gn.Tag = g;
                    foreach (MemoItem item in g.Items)
                    {
                        TreeNode inode = gn.Nodes.Add(item.Name);
                        inode.Tag = item;
                        inode.ToolTipText = item.Cmd;
                    }
                }
                tree.ExpandAll();
            }
            finally
            {
                tree.EndUpdate();
            }
            ApplyLayout();
        }

        private MemoGroup GroupOf(TreeNode node)
        {
            if (node == null) return null;
            if (node.Tag is MemoGroup) return (MemoGroup)node.Tag;
            if (node.Tag is MemoItem && node.Parent != null) return node.Parent.Tag as MemoGroup;
            return null;
        }

        private MemoItem SelectedItem
        {
            get { return tree.SelectedNode == null ? null : tree.SelectedNode.Tag as MemoItem; }
        }

        private void NotifyChanged()
        {
            if (Changed != null) Changed();
        }

        // ---------------- 操作 ----------------

        private void OnTreeKeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Enter)
            {
                RunSelected(true);
                e.Handled = true;
            }
        }

        private void OnNodeDoubleClick(object sender, TreeNodeMouseClickEventArgs e)
        {
            if (e.Node != null && e.Node.Tag is MemoItem)
            {
                RunSelected(true);
            }
        }

        private void RunSelected(bool execute)
        {
            MemoItem item = SelectedItem;
            if (item == null || RunCommand == null) return;
            RunCommand(item.Cmd, execute);
        }

        private void OnCopyCommand(object sender, EventArgs e)
        {
            MemoItem item = SelectedItem;
            if (item == null) return;
            try
            {
                Clipboard.SetText(item.Cmd);
            }
            catch
            {
            }
        }

        private List<string> GroupNames()
        {
            var names = new List<string>();
            foreach (MemoGroup g in groups)
            {
                if (g.Group.Length > 0) names.Add(g.Group);
            }
            return names;
        }

        private void OnAddItem(object sender, EventArgs e)
        {
            MemoGroup g = GroupOf(contextNode != null ? contextNode : tree.SelectedNode);

            var dlg = new MemoEditDialog("新建命令", "", "",
                GroupNames(), g == null ? "" : g.Group);
            if (dlg.ShowDialog(this) != DialogResult.OK) return;

            // 分组为空或找不到时，落到第一组或新建一个
            if (dlg.GroupValue.Length > 0 && (g == null || g.Group != dlg.GroupValue))
            {
                g = null;
                foreach (MemoGroup grp in groups)
                {
                    if (grp.Group == dlg.GroupValue) { g = grp; break; }
                }
                if (g == null)
                {
                    g = new MemoGroup();
                    g.Group = dlg.GroupValue;
                    groups.Add(g);
                }
            }
            if (g == null)
            {
                if (groups.Count == 0)
                {
                    g = new MemoGroup();
                    g.Group = "常用";
                    groups.Add(g);
                }
                else
                {
                    g = groups[0];
                }
            }

            var item = new MemoItem();
            item.Name = dlg.NameValue;
            item.Cmd = dlg.CmdValue;
            g.Items.Add(item);

            SetGroups(groups);
            NotifyChanged();
        }

        private void OnEditItem(object sender, EventArgs e)
        {
            MemoItem item = SelectedItem;
            MemoGroup oldGroup = GroupOf(tree.SelectedNode);
            if (item == null || oldGroup == null) return;

            var dlg = new MemoEditDialog("编辑命令", item.Name, item.Cmd,
                GroupNames(), oldGroup.Group);
            if (dlg.ShowDialog(this) != DialogResult.OK) return;

            item.Name = dlg.NameValue;
            item.Cmd = dlg.CmdValue;

            // 换了分组就移动
            if (dlg.GroupValue.Length > 0 && dlg.GroupValue != oldGroup.Group)
            {
                MemoGroup target = null;
                foreach (MemoGroup grp in groups)
                {
                    if (grp.Group == dlg.GroupValue) { target = grp; break; }
                }
                if (target == null)
                {
                    target = new MemoGroup();
                    target.Group = dlg.GroupValue;
                    groups.Add(target);
                }
                oldGroup.Items.Remove(item);
                target.Items.Add(item);
            }

            SetGroups(groups);
            NotifyChanged();
        }

        private void OnDeleteItem(object sender, EventArgs e)
        {
            MemoGroup g = GroupOf(tree.SelectedNode);
            MemoItem item = SelectedItem;
            if (g == null || item == null) return;

            g.Items.Remove(item);
            SetGroups(groups);
            NotifyChanged();
        }

        private void OnNewGroup(object sender, EventArgs e)
        {
            var dlg = new InputDialog("新建命令分组", "分组名称：", "");
            if (dlg.ShowDialog(this) != DialogResult.OK) return;
            if (dlg.Value.Length == 0) return;

            var g = new MemoGroup();
            g.Group = dlg.Value;
            groups.Add(g);

            SetGroups(groups);
            NotifyChanged();
        }

        private void OnRenameGroup(object sender, EventArgs e)
        {
            MemoGroup g = GroupOf(tree.SelectedNode);
            if (g == null) return;

            var dlg = new InputDialog("重命名分组", "分组名称：", g.Group);
            if (dlg.ShowDialog(this) != DialogResult.OK) return;
            if (dlg.Value.Length == 0) return;
            g.Group = dlg.Value;

            SetGroups(groups);
            NotifyChanged();
        }

        private void OnDeleteGroup(object sender, EventArgs e)
        {
            MemoGroup g = GroupOf(tree.SelectedNode);
            if (g == null) return;

            string name = g.Group.Length > 0 ? g.Group : "(未分组)";
            if (MessageBox.Show(this, "删除分组「" + name + "」及其下所有命令？", "确认",
                MessageBoxButtons.YesNo, MessageBoxIcon.Question) != DialogResult.Yes)
            {
                return;
            }
            groups.Remove(g);
            SetGroups(groups);
            NotifyChanged();
        }
    }
}
