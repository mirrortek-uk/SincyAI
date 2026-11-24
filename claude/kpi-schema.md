# Sincy 数字员工 · 全平台 KPI / 配额规格 (v1)

> 目标:让你能给每个运营账号「下 KPI」。一次覆盖所有已知平台,新增平台按模板扩展。
> 设计原则:**两类 KPI 分开** —— 活动配额(你下,系统必达,受健康约束)vs 增长目标(你设,系统尽力)。所有活动配额都在「健康红线」之下,数值给的是**克制、人类合理**的起点,可调。不做靠多号/反检测规模化刷量的那套。

---

## 1. 通用 schema(所有平台共用的形状)

两级生效:**平台模板默认值 ⊕ 员工账号覆盖 = 本账号实际 KPI**。
新增平台 = 建一行平台模板;给某账号加力 = 在员工 strategy 上覆盖几个字段。

```jsonc
// 挂在 employee_strategies.strategy.kpi (或新表 account_kpi), 按平台
{
  "platform": "x",
  "category": "text_social",            // 决定可用动作与默认档

  // ───────── A 活动配额(你下 · 必达 · 受 D 约束) ─────────
  "activity_quotas": {
    "posts_per_day": 3,
    "posts_per_week": null,             // 长视频平台用周粒度, 则 per_day 置 null
    "replies_per_day": 12,             // 回时间线 + 回自己帖下评论, 合计
    "reply_own_mentions_ratio": 0.5,   // 其中回"别人@我/评论我"的占比
    "follows_per_day": 15,
    "likes_per_day": 30,
    "reposts_per_day": 3,              // 转发/引用(平台无则忽略)
    "saves_per_day": null,             // 收藏(图文平台)
    "duet_stitch_per_day": null,       // 短视频二创
    "dms_per_day": 0,                  // 默认 0, 防骚扰
    "posting_windows": ["09:00-11:00","19:00-21:00"],
    "timezone": "Europe/Bucharest",
    "fanout_platforms": ["x"]          // 这条母稿同步分发到哪些平台
  },

  // ───────── B 质量/合规门(约束 · 过不了不发) ─────────
  "quality_gates": {
    "min_quality_score": 60,
    "require_fact_check_pass": true,
    "max_duplicate_ratio": 0.2,
    "forbidden_topics_enforced": true,
    "ai_label_required": true
  },

  // ───────── C 增长目标(你设 · 尽力 · 不保证) ─────────
  "growth_targets": {
    "metric": "followers",             // followers / subscribers / 粉丝 / karma
    "target": 10000,
    "horizon_days": 90,
    "daily_net_growth_min": 30,        // 期望日净增 → 落后则在配额/健康内加力
    "engagement_rate_target": 0.03,    // 互动率目标
    "secondary": {}                    // 平台特有: 完播率/观看时长/CTR/收藏率…
  },

  // ───────── D 健康红线/止损(保号 · 自动降速/停) ─────────
  "health_guardrails": {
    "max_actions_per_hour": 20,        // 全动作总限速(礼貌)
    "daily_action_ceiling": 80,        // 所有动作/天总上限(防封)
    "stop_on_shadowban": true,
    "stop_on_rate_limit": true,
    "stop_on_policy_strike": true,
    "cooldown_minutes_after_block": 120,
    "challenge_to_human": true         // 人机验证一律转人工, 从不自动过
  }
}
```

**字段语义约定**
- `*_per_day = 0` 表示明确禁用该动作;`null` 表示该平台无此动作(模板里就不出现)。
- A 的任何配额都被 D 的 `max_actions_per_hour` / `daily_action_ceiling` 压顶:**配额再高,也不会突破健康上限**。
- C 不可作为"必达"下达;系统只把它当优化方向 + 进度展示 + 学习 reward。

---

## 2. 平台分类(模板档位)

平台先归到一个 `category`,继承该档的**可用动作集**与**默认配额量级**;再按单平台微调。

| category | 典型平台 | 内容节奏 | 核心动作 | 增长指标 | 互动构成(engagement) |
|---|---|---|---|---|---|
| `text_social` | X, Threads, Weibo, Bluesky, Mastodon | 高(日多帖) | post/reply/repost/quote/like/follow | followers/粉丝 | (赞+评+转)/曝光 |
| `pro_social` | LinkedIn | 低(周 3-5) | post/comment/react/connect/follow | followers | (赞+评+转)/曝光 |
| `image_lifestyle` | 小红书, Instagram(feed), Pinterest | 中(日 1-2) | post/comment/like/**save收藏**/follow | 粉丝/followers | (赞+评+**收藏**+转)/曝光 |
| `short_video` | TikTok, Reels, Shorts, 抖音, 快手 | 中(产能受限,日 1-2) | post(video)/comment/like/follow/duet/stitch | followers/粉丝 | (赞+评+分享+收藏)/播放 + **完播率** |
| `long_video` | YouTube, Bilibili | 低(**周** 2-4) | publish/community_post/comment/reply/like | subscribers/粉丝 | **观看时长 + 留存 + CTR** + 赞/评 |
| `community` | Reddit | 中但受自荐规则约束 | post/comment/upvote/crosspost | karma/关注 | upvote 比 + 评论 |

---

## 3. 各平台 KPI 画像(默认值 = 克制起点,可调)

> 默认配额是**安全的人类节奏**,不是增长上限。想更激进请调高,但 D 的健康止损始终兜底。

### 3.1 X (Twitter) · `text_social`
- 增长:`followers` · 互动:(likes+replies+reposts)/impressions
- 动作:post(含 thread)、reply、repost、quote、like、follow、dm
- 默认/天:posts **3** · replies **12** · follows **15** · likes **30** · reposts **3** · quotes **1** · dms **0**
- 健康:shadowban(接 shadowban-radar)、reply/search 可见性、429 限流 → 降速
- 二级目标:tweet_impressions、profile_visits

### 3.2 小红书 XHS · `image_lifestyle`
- 增长:`粉丝` · 互动:(点赞+评论+**收藏**+分享)/曝光 ——**收藏权重高**
- 动作:post(图文/视频笔记)、comment、like、**save收藏**、follow、私信
- 默认/天:posts **1-2** · comments **12** · follows **15** · likes **25** · 私信 **0**
- 二级目标:**收藏率**、笔记进入推荐池比例、搜索排名
- 健康:限流/违规/笔记被折叠、私信频控 → 降速;硬广/导流词触发合规门

### 3.3 TikTok · `short_video`
- 增长:`followers` · 互动:(likes+comments+shares+saves)/views + **完播率**
- 动作:post(video)、comment、like、follow、duet、stitch、share
- 默认/天:posts **1-2** · comments **15** · follows **15** · likes **30** · duet/stitch **1**
- 二级目标:**完播率(completion)**、平均观看时长、FYP 进入率、分享数
- 健康:低流量池(FYP 抑制)、社区违规、完播过低 → 调整;避免搬运/重复音轨

### 3.4 YouTube · `long_video`
- 增长:`subscribers` · 真·KPI:**观看时长(watch time)、留存(retention)、CTR**
- 动作:publish(video)、community_post、comment、reply(回自己视频评论,拉留存)、like
- 默认:posts **2-4 / 周**(非每天)· community_post **0-1/天** · comments/replies **15/天** · likes **15/天**
- 二级目标:CTR、平均观看时长、留存曲线、订阅转化(watch→sub)
- 健康:社区准则 strike、版权、限流 → 止损

### 3.5 Instagram · `image_lifestyle`(+ Reels 走 `short_video`)
- 增长:`followers` · 互动:(likes+comments+saves+shares)/reach
- 动作:post(feed)、reels、story、comment、like、save、follow、dm
- 默认/天:feed posts **1** · reels **0-1** · stories **1-3** · comments **12** · follows **15** · likes **30** · dms **0**
- 二级目标:saves、reach、profile_visits、reels 完播
- 健康:action block(关注/点赞过频)、shadowban(标签)、社区违规 → 降速

### 3.6 微博 Weibo · `text_social`
- 增长:`粉丝` · 互动:(赞+评论+转发)/阅读
- 动作:post(微博)、评论、转发、赞、关注
- 默认/天:posts **3-5** · 评论 **12** · 关注 **15** · 赞 **30** · 转发 **3**
- 二级目标:阅读数、转发链、上热搜/话题榜
- 健康:限流、违规禁言、关注频控 → 降速

### 3.7 抖音 Douyin · `short_video`
- 增长:`粉丝` · 互动:(赞+评论+分享+收藏)/播放 + 完播率
- 动作:post(video)、评论、点赞、关注、私信
- 默认/天:posts **1-2** · 评论 **15** · 关注 **15** · 点赞 **30** · 私信 **0**
- 二级目标:完播率、平均时长、推荐池进阶、分享
- 健康:限流/违规、搬运检测、完播过低 → 调整

### 3.8 快手 Kuaishou · `short_video`
- 同抖音档;增长 `粉丝`;更重**老铁互动/评论区经营**,可把 comments/replies 调高(**20/天**)。

### 3.9 Bilibili · `long_video`
- 增长:`粉丝` · 互动:**三连(点赞+投币+收藏)** + 评论 + 弹幕 + 观看时长
- 动作:publish(视频/动态)、评论、点赞、投币、收藏、关注、弹幕
- 默认:视频 **2-4/周** · 动态 **0-1/天** · 评论 **15/天** · 关注 **12/天**
- 二级目标:**三连率**、完播、弹幕密度、追番/关注转化
- 健康:违规、限流、搬运 → 止损

### 3.10 LinkedIn · `pro_social`
- 增长:`followers`/connections · 互动:(react+comment+repost)/impressions
- 动作:post(文章/动态)、comment、react、**connect(邀请)**、follow
- 默认:posts **3-5/周**(非每天)· comments **8/天** · connects **10-15/天**(**周上限 ~100**)· reacts **20/天**
- 二级目标:impressions、profile_views、connect 接受率
- 健康:**连接邀请周配额**(超了被限)、受限审查 → 降速

### 3.11 Threads / Bluesky / Mastodon · `text_social`
- 同 X 档;增长 `followers`;动作 repost 在 Mastodon 叫 boost;默认配额照 X,略保守(新平台生态小):posts **2-3** · replies **10** · follows **12**。

### 3.12 Pinterest · `image_lifestyle`
- 增长:`followers`(次要)· 真·KPI:**pin 的曝光/保存/外链点击**
- 动作:pin(post)、comment、**save/repin**、follow
- 默认/天:pins **3-5** · repins **5** · follows **10** · comments **5**
- 二级目标:pin impressions、saves、outbound_clicks

### 3.13 Reddit · `community`
- 增长:`karma` + subreddit 立足;follower 次要 · 互动:upvote 比 + 评论质量
- 动作:post(到 subreddit)、comment、upvote、crosspost
- 默认/天:posts **1-2**(严守各 sub 自荐规则,**遵循 9:1 价值:自荐**)· comments **10** · upvotes 随手
- 二级目标:upvote ratio、评论 karma、未被移除率
- 健康:**shadowban、账号年龄/karma 门槛、subreddit ban、自荐比例**(超标即违规)→ 极保守

### 3.14 Facebook Page · `text_social`(偏主页)
- 增长:`粉丝/关注` · 互动:(reactions+comments+shares)/reach
- 动作:post、comment/reply、react、share
- 默认/天:posts **1-2** · comments **10** · 不主动加好友(主页场景)
- 二级目标:reach、page_views、shares

---

## 4. 新平台扩展模板(照抄即接)

新增平台 = 填这张 stub,`category` 定了就继承该档默认动作与量级,再覆盖差异:

```jsonc
{
  "platform": "<slug>",              // 如 "lemon8"
  "category": "image_lifestyle",     // 选 §2 的一档
  "growth_metric": "followers",
  "available_actions": ["post","comment","like","save","follow"],
  "engagement_formula": "(likes+comments+saves+shares)/impressions",
  "default_quotas": {                // 不写则继承 category 档
    "posts_per_day": 1, "comments_per_day": 10, "follows_per_day": 12, "likes_per_day": 20
  },
  "secondary_metrics": ["saves","reach"],
  "health_signals": ["rate_limit","policy_violation","throttle"],
  "rate_caps": { "max_actions_per_hour": 15, "daily_action_ceiling": 60 },
  "adapter_requirements": [          // 对应要补的设备适配器
    "post", "comment", "like", "save", "follow", "read_followers", "read_mentions"
  ]
}
```

扩展检查清单:① 填模板 → ② 建 `channel_capabilities` 行(platform × action × route)→ ③ 写/核验设备 YAML 适配器(含 `read_followers` 读数)→ ④ (若有)接该平台增长/健康 provider。

---

## 5. 循环如何消费这套 KPI(接线要点)

1. **排程 → 日计划**:调度器按 `activity_quotas` + `posting_windows` 为每个账号算出"今天该发 N 帖、回 M 条、关注 K 个,在哪些时段",把配额摊进任务。
2. **生成任务**:发帖走内容工厂(Brief→工序);回复/关注走**互动子循环**(read_mentions/read_timeline → 选目标 → reply/follow 任务)。
3. **达成跟踪**:`account_activity_ledger(date, platform, action, count)` 记录当天已发/已回/已关,对配额"缺则补、满则停"。
4. **健康止损**:D 层信号(shadowban/限流/strike)命中 → 当日降速或暂停该账号,进 cooldown;人机验证转人工。
5. **增长回流**:`account_metric_samples` 的 Δ 粉丝/互动 → C 层进度 + 学习 reward(朝涨粉优化)。
6. **优先级**:健康(D)> 合规(B)> 活动配额(A)> 增长加力(C)。即:先保号、再守质量、再完成配额、最后在余量内为目标加力。

---

## 6. 边界

- 所有默认配额是**克制的人类节奏**,并被健康上限兜底;**不设计靠多小号、指纹伪装、绕限频/验证码的规模化刷量**——那是封号高发区,也不在本规格范围。
- 活动配额"必达"的前提永远是"健康允许";任何平台检测到限流/违规,**降速保号优先于完成配额**。
