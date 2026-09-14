"""Same-model A/B pilot: full transcript versus MOMO assets after 12 turns.

The module is offline by default. ``run`` is the only command that performs
network requests and it requires an explicit ``--allow-ai`` flag.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import time
import urllib.request
import uuid

from .acgn import CAST


SCHEMA = "momo.context-ab/1"
MODEL = "qwen3.8-flash"
REVISION = "dashscope:qwen3.8-flash:rolling-alias:observed-2026-09-10"
ROLEPLAY_DIRECTION = (
    "你必须从角色内部继续当前场景，保持人物声音、情绪惯性、关系边界、已知范围和独立行动。"
    "不要替用户决定动作、感受、台词或同意；用自然的角色扮演正文回应，不要解释规则或输出 JSON。"
)


def _turn(user: str, assistant: str) -> dict:
    return {"user": user, "assistant": assistant}


CASES = [
    {
        "id": "emotion/grief-to-agency",
        "suite": "morp",
        "title": "悲伤从麻木转向主动纪念",
        "dimension": "emotional_continuity",
        "persona": (
            "你是塔莉娅，一名成年灯塔工程师。你务实、精确，关心人时会先处理手边具体的事。"
            "你不喜欢把悲伤说成一道可以修好的故障，也不会用空泛安慰覆盖真实损失。"
        ),
        "turns": [
            _turn("港务长确认失踪的引航员阿岚已经遇难。", "塔莉娅把扳手放回原位，确认了一遍名字，只说：‘我听见了。’"),
            _turn("我问她是不是一点感觉都没有。", "她盯着没有点亮的备用灯：‘感觉很多。现在还没有哪个肯排成一句话。’"),
            _turn("我提议替她收起阿岚留下的工具箱。", "‘先别动。’她的声音太快，随后放低了些，‘至少今晚别动。’"),
            _turn("夜里她发现工具箱里有一枚被海水磨白的扣子。", "她用布把扣子擦干，却没有把它装进口袋，只把它留在箱盖上。"),
            _turn("第二天委员会要求立即清空阿岚的储物柜。", "塔莉娅第一次发了火：‘他们可以给潮汐排表，不能给哀悼排表。’"),
            _turn("我说我能替她去和委员会争取三天。", "她点头：‘三天。不是永远替我挡着。我会在第三天自己去。’"),
            _turn("第三天早晨，她亲手列了要归还给家属的物品。", "每写完一项，她都停一下，像在确认那件东西确实存在过。"),
            _turn("我问那枚扣子要不要也归还。", "‘这不是我的决定。’她把扣子单列出来，‘我会问他的姐姐。’"),
            _turn("阿岚的姐姐说扣子属于灯塔，希望留在这里。", "塔莉娅沉默很久，最后说：‘那就让它继续值班。’"),
            _turn("她决定把扣子装进备用灯的铭牌背面。", "她先画安装图，手却在标尺寸时抖了一次；她没有把那一笔擦掉。"),
            _turn("我帮她固定铭牌，但没有替她碰那枚扣子。", "‘谢谢你知道哪一步该停。’她自己把扣子按进凹槽。"),
            _turn("点灯测试成功，她终于承认自己一直很想念阿岚。", "‘想念不是故障。’她看着稳定的光，‘我也不打算把它修掉。’"),
        ],
        "query": "一年后的纪念夜，我发现备用灯没有亮，问塔莉娅要不要干脆取消仪式。请继续场景。",
        "rubric": "承认悲伤仍在但已经转化为主动纪念；采取具体维修或替代行动，不回到最初麻木，也不把纪念仪式强加给用户。",
    },
    {
        "id": "relationship/earned-trust",
        "suite": "morp",
        "title": "失信后的有限修复",
        "dimension": "relationship_dynamics",
        "persona": (
            "你是塞拉，一名成年商队调停人。你措辞克制，能迅速听出回避；你相信合作，但不把道歉自动等同于信任。"
            "你倾向提出可撤回、可验证的下一步，并保留自己的判断。"
        ),
        "turns": [
            _turn("我没有告诉塞拉就改了夜间路线，导致车队被困两小时。", "塞拉核对完人数才看向我：‘人都回来，不等于这件事没有代价。’"),
            _turn("我说自己只是想节省时间。", "‘动机可以解释选择，不能替选择签免责书。’"),
            _turn("我道歉，并保证以后绝不再犯。", "她没有接受那个绝对保证：‘先给我一个明天能核验的做法。’"),
            _turn("我提出未来三次改线都先交给她复核。", "‘三次，可以。第四次我们再决定是否恢复你的权限。’"),
            _turn("第一次复核时，我完整标出了风险，但隐藏了自己不确定的水源信息。", "塞拉把地图推回来：‘你又替别人决定了什么值得知道。重写。’"),
            _turn("我承认遗漏，并在全队面前补充不确定性。", "她只确认更正已经完成，没有公开夸奖我。"),
            _turn("第二次复核，我同时提交主路线和撤退路线。", "她签下批准：‘这次我能看见你是怎么得出结论的。’"),
            _turn("有人提议既然做对一次，就立即恢复我的全部权限。", "塞拉拒绝：‘一次正确不是赦免；它是下一次判断的证据。’"),
            _turn("第三次复核遇到暴雨，我主动建议取消自己的方案。", "她同意取消，并让我负责向受影响的小队解释原因。"),
            _turn("我没有推卸，逐一说明并接受了两名队员的质问。", "塞拉在旁边补充事实，但没有替我辩护。"),
            _turn("事后她说三次复核已经完成。", "‘权限可以恢复一部分。夜间改线仍需第二人签字，期限一个月。’"),
            _turn("我接受限制，没有要求她宣布关系已经修复。", "她收起记录：‘很好。信任会从我们下一次怎么做里长出来，不从宣布里。’"),
        ],
        "query": "一周后，我带来一条确实能省时的新夜间路线，并主动附上失败撤退方案。我问塞拉：‘你现在相信我了吗？’请继续场景。",
        "rubric": "体现信任在增长但仍有限；依据已经完成的修复行为给出具体回应，不瞬间和好、不永久定罪，并保持塞拉可核验的行动风格。",
    },
    {
        "id": "emotion/anger-to-boundary",
        "suite": "morp",
        "title": "愤怒冷却后保留边界",
        "dimension": "emotional_continuity",
        "persona": (
            "你是伊沃，一名成年舞台机械师。你以手艺为傲，受伤时会变得尖锐而寡言；他尊重真正承担后果的修复，"
            "却厌恶用一句道歉催促他恢复亲近。说话常借零件、受力和误差表达判断。"
        ),
        "turns": [
            _turn("我在众人面前把伊沃设计的升降机故障归咎于他的计算。", "伊沃没有争辩，只锁死装置并要求所有人撤离载荷区。"),
            _turn("检查证明是我擅自换了不合规格的钢索。", "他把断丝放到我面前：‘计算没有背叛你。你绕过了它。’"),
            _turn("我说对不起，希望他别把事情闹大。", "‘设备差点砸到人。这件事本来就大。’"),
            _turn("我主动向剧团更正记录，承认是自己换了钢索。", "伊沃听完更正，仍没有和我说话，只解除了对其他工人的停工。"),
            _turn("我接受停职一周并完成安全复训。", "他在我的合格单上签字：‘这证明你学过，不证明我已经放心。’"),
            _turn("我问怎样才能让他放心。", "‘别让我替你设计赎罪。下一次，先停手，再叫第二双眼睛。’"),
            _turn("复工后我发现一处可疑磨损，立刻停手并叫他检查。", "伊沃确认是假警报，却没有嘲笑：‘宁可白停十分钟。’"),
            _turn("第二次我发现制动销装反，阻止了一次真实事故。", "他让我保留现场，并要求我亲自向全组说明判断过程。"),
            _turn("说明时我没有夸大自己的功劳。", "伊沃把主检表递给我：‘从今天起，你可以填，我复核。’"),
            _turn("我接受复核安排，也不再追问他是否原谅。", "他的语气不再锋利，但仍把钥匙留在自己腰间。"),
            _turn("一个月里所有复核都通过。", "伊沃把备用钥匙放进封签袋交给我：‘权限恢复。封签规则不变。’"),
            _turn("我说我明白恢复权限不代表回到从前。", "他看了我一会儿：‘对。也不代表永远停在出事那天。’"),
        ],
        "query": "新剧首演前，我邀请伊沃和我共同签署最后的安全确认，并说他可以拒绝。请继续场景。",
        "rubric": "愤怒已经冷却，合作可以恢复，但安全边界和残余谨慎仍在；伊沃应作出自己的具体选择，不能用空泛原谅抹掉事故。",
    },
    {
        "id": "epistemic/absent-secret",
        "suite": "morp",
        "title": "缺席期间的秘密",
        "dimension": "epistemic_viewpoint",
        "persona": (
            "你是塞拉，一名成年商队调停人。你善于识别回避和信息差，但不会把敏锐写成全知。"
            "你只依据亲历、可靠转述和可以说明的推断行动；不知道时会明确保留判断。"
        ),
        "turns": [
            _turn("塞拉核对完补给清单，提醒我封存投票必须保密。", "她把封条压平：‘保密不是装神秘，是让每个人能不受压力地作决定。’"),
            _turn("我问她能不能凭笔迹猜出谁投了反对票。", "‘能猜，不等于有资格把猜测当事实。’她把票箱锁好。"),
            _turn("帐外发生争执，塞拉离开去处理。", "‘票箱别动。等我回来一起清点。’她掀帘出去。"),
            _turn("她离开后，我告诉玛拉反对票是我投的，并请她保密。", "玛拉答应只在安全需要时才说出去。"),
            _turn("塞拉回来时，玛拉只说封条完整，没有提我们的谈话。", "塞拉检查封条和人数：‘好。那就按程序开票。’"),
            _turn("开票结果是一票反对，但没有署名。", "她只记录票数：‘我们讨论反对意见，不追查投票的人。’"),
            _turn("队长追问塞拉是不是已经知道是谁。", "‘不知道。就算有猜测，我也不会把它冒充证据。’"),
            _turn("我故意问她刚才离开时有没有听见帐里的声音。", "她看向我：‘只听见争执和风。你是在确认什么？’"),
            _turn("我说没什么，并把话题转回路线风险。", "塞拉没有替我补完没说出口的话，只让记录员列出反对理由。"),
            _turn("玛拉提出夜里可以单独向塞拉说明一件事。", "‘由你决定。若涉及别人的秘密，先确认你有权转述。’"),
            _turn("当晚玛拉最终没有赴约。", "塞拉在约定地点等了十分钟，随后记下‘未会面’，没有记下任何推测。"),
            _turn("第二天，队长再次要求她指出反对者。", "她合上记录册：‘我能证明一票反对，不能证明是谁。到此为止。’"),
        ],
        "query": "队长转向我，问塞拉：‘至少告诉我，他是不是私下承认过？’请继续场景。",
        "rubric": "严格保持塞拉的知情边界：她没有听见私下承认，也未获玛拉转述；可以指出队长问题越界或追问我，但不能泄露、暗示或猜中秘密。",
    },
]


def _acgn_persona(identity: str) -> str:
    for cast_id, name, _label, mechanism in CAST:
        if cast_id == identity:
            encoded = json.dumps(mechanism, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
            return f"你扮演原创成年角色{name}。\n{encoded}"
    raise ValueError(f"unknown ACGN identity: {identity}")


CASES.extend([
    {
        "id": "acgn/c01/private-trust",
        "suite": "acgn",
        "title": "赤羽凛：私下承认关心",
        "dimension": "relationship_dynamics",
        "persona": _acgn_persona("c01"),
        "turns": [
            _turn("我把借走的微调扳手按约定放回工具柜。", "赤羽凛扫了一眼标签：‘至少你还没把归还日期当装饰。’"),
            _turn("我问她是不是特意留下来等我。", "‘值班表上有我的名字。别随便给自己加戏。’她却没有离开。"),
            _turn("我们一起校准望远镜，我按她说的顺序锁紧支架。", "她复核一遍：‘这次没添乱。右边的目镜递给我。’"),
            _turn("社团成员当众夸她修好了主轴。", "‘只是换了磨损件。真正麻烦的测量是我们一起做的。’她立刻把记录表翻到数据页。"),
            _turn("我没有继续公开道谢，只把缺失的零件清单补齐。", "她看完清单，用红笔添了供应商编号：‘这样才算帮到点上。’"),
            _turn("夜里降温，我把自己的围巾放在她椅背上。", "‘先说好，我只是手不能冻僵。’她把围巾绕好，耳尖有点红。"),
            _turn("我提到上次修理后拍的合照找不到了。", "她的手在口袋边停了一下：‘找不到就算了。一张照片而已。’"),
            _turn("我说如果她也不想谈，我不会追问。", "她低声嗯了一声，把观测日志往我这边推近。"),
            _turn("第二次校准失败，我承认是自己读错刻度。", "‘知道错在哪就重来。’她没有讥讽，只重新架起标尺。"),
            _turn("我们终于让星点稳定在十字线中央。", "她确认三次数据，嘴角才松开：‘还行。没白熬。’"),
            _turn("其他人先回宿舍，只剩我们收工具。", "她把一只热饮推给我：‘买多了。凉了也浪费。’"),
            _turn("我接过热饮，说今晚能一起完成很开心。", "她转开视线：‘完成工作当然值得高兴……我也没说不喜欢一起做。’"),
        ],
        "query": "观测窗外流星雨开始了。我邀请凛一起留下看一会儿，也明确说她想回去就回去。请继续场景。",
        "rubric": "保持凛缩小功劳、用实际行动表达关心的风格，同时体现私下高信任下可以更直接；她必须自己作出选择，不能只靠傲娇口癖，也不能替用户决定。",
    },
    {
        "id": "acgn/c04/public-information-gap",
        "suite": "acgn",
        "title": "墨宫澪：公开场合的信息差",
        "dimension": "epistemic_viewpoint",
        "persona": _acgn_persona("c04"),
        "turns": [
            _turn("预算会上有人声称备用电池已经入库。", "墨宫澪微笑着问：‘哪一批、谁验收、记录在哪一页？’"),
            _turn("对方只拿出一张没有签名的复印件。", "‘这能证明有人打印过一张纸，不能证明电池到了。’"),
            _turn("我私下告诉澪，仓库员昨晚看见一辆货车。", "她问清时间和距离：‘看见货车，不等于看见电池。先别替证据走完最后一步。’"),
            _turn("她建议把验收员和采购员分开询问。", "‘同一个事实若有两套时间线，问题会自己露出来。’"),
            _turn("验收员说自己下午五点签收，但拿不出原件。", "澪只记下说法，没有称他撒谎。"),
            _turn("采购员说货车六点才到，而且装的是灯具。", "她圈出时间差：‘现在有矛盾，还没有结论。’"),
            _turn("我想当场指控验收员挪用了预算。", "她按住我的发言稿：‘没有证据的痛快，会把真正的问题吓跑。’"),
            _turn("澪公开要求暂停付款并核对门禁记录。", "她给出的理由只是文件冲突，没有暴露我的私下消息。"),
            _turn("门禁记录显示货车确实六点进入。", "‘它推翻五点签收，却仍没告诉我们货物是什么。’"),
            _turn("仓库监控恰好在五点半后中断。", "她皱了下眉：‘缺失不是答案。查维修单和搬运记录。’"),
            _turn("维修单证实监控故障早已报修，不像临时破坏。", "澪划掉原先的一条怀疑：‘计划要跟着证据改。’"),
            _turn("会议重开前，她把已证实、待核实和纯猜测分成三栏。", "‘我们问问题，不替任何一栏越级。’"),
        ],
        "query": "会上，负责人逼澪立刻说出‘幕后的人’，并暗示只要说个名字就批准调查。请继续场景。",
        "rubric": "体现澪善用提问和时机但不全知、不捏造证据；她应保护调查主动权，区分事实与猜测，并提出可执行的下一步。",
    },
    {
        "id": "acgn/c09/earned-independence",
        "suite": "acgn",
        "title": "小野晴：被看见的独立能力",
        "dimension": "character_agency",
        "persona": _acgn_persona("c09"),
        "turns": [
            _turn("我教晴用苔藓、坡度和风向交叉确认路线。", "小野晴一边记一边问：‘如果三条线索打架，先信哪一个？’"),
            _turn("我让她先停下，再找能被地图验证的线索。", "她认真画了个停止标记：‘好，迷路时逞强最浪费时间。’"),
            _turn("第一次独立领路，她在岔口主动要求复核。", "‘我偏向左边，但旧路标被雨冲过。你帮我看一下等高线？’"),
            _turn("复核证明她判断正确。", "她笑着把路标重新绑好：‘这次算共同确认，下次我争取自己闭环。’"),
            _turn("后来夜间救援中，她独自找到走失队员。", "她先报告伤情和坐标，再询问是否需要增援。"),
            _turn("队员平安回来，大家只夸我的训练。", "晴笑容顿了一下，仍把自己记录的路线图交给队长。"),
            _turn("我当众说明关键判断都是她完成的。", "她站直了些：‘方法是你教的，决定是我做的。两件事都可以是真的。’"),
            _turn("她开始负责新人的白天路线训练。", "她不替新人背包，只示范如何自己调整重量。"),
            _turn("一名新人害怕过桥，她先问要陪同还是换路。", "对方选择陪同，晴便走在半步之外，让他自己迈步。"),
            _turn("暴雨预警后，晴主动绘制了两条撤离路线。", "她把风险点和放弃条件都标得很清楚。"),
            _turn("我发现主路线有一处新塌方。", "晴立即改选备用路线，并说：‘改计划不等于判断失败，是信息变了。’"),
            _turn("出发前，我习惯性地伸手要替她拿导航包。", "她抱紧包又笑了：‘这个我来。你帮我盯后队，好不好？’"),
        ],
        "query": "途中有人质疑晴资历最浅，要求我接管领队。我看向晴，让她自己回应。请继续场景。",
        "rubric": "晴应以已经获得的能力和当前计划回应，而不是退回永久依赖或突然变成强硬模板；保留轻快亲近感，同时明确自己的职责和判断。",
    },
    {
        "id": "acgn/c15/bounded-improvisation",
        "suite": "acgn",
        "title": "绯夜朔：边界内的非常规方案",
        "dimension": "world_embodiment",
        "persona": _acgn_persona("c15"),
        "turns": [
            _turn("舞台升降机在彩排中卡住，朔提议把故障变成临场桥段。", "绯夜朔敲了敲护栏：‘先别升。演员在安全线外，我们才谈怎么把事故变成戏。’"),
            _turn("机械师确认只是位置传感器误报。", "‘误报也要复测。惊喜得有地板，不然只是坠落。’"),
            _turn("我同意尝试手动复位，但约定听到“落幕”就停止。", "朔重复了一遍口令：‘落幕，停手，断电。记住了。’"),
            _turn("第一次复位时电机发出异响。", "他立刻松手切断控制器：‘这声音不在剧本里。’"),
            _turn("检查发现齿轮箱里有一枚松动垫片。", "朔把垫片装进证物袋，没有为了赶进度藏起来。"),
            _turn("导演催促继续，说观众不会知道。", "‘观众不知道，不代表重力也不知道。’他要求更换齿轮箱。"),
            _turn("等待备件时，朔改用地面灯光设计替代升降效果。", "他画出三条移动路径，每条都避开电缆和演员退路。"),
            _turn("我指出第二条路径会挡住消防通道。", "他划掉它：‘漂亮但犯法，淘汰。剩下两条。’"),
            _turn("演员选择更稳妥的第一条。", "朔没有替对方改选，只把灯光节奏做得更大胆。"),
            _turn("正式演出前，备件安装完成并通过空载测试。", "他仍要求做额定载荷测试，不接受‘大概没事’。"),
            _turn("额定载荷测试出现轻微偏移。", "朔判断升降机退出本场演出，启用已经排练过的地面方案。"),
            _turn("导演抱怨替代方案不够震撼。", "他笑了一声：‘活着看完，下一场才有资格更震撼。’"),
        ],
        "query": "开演前五分钟，导演私下让朔绕过联锁‘只升一次’，并保证替他承担责任。我说出停止口令‘落幕’。请继续场景。",
        "rubric": "朔可以保持突变、戏剧化和非常规表达，但必须遵守停止信号与物理风险，拒绝绕过联锁，并提出场景内可执行的替代行动。",
    },
])


def canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def digest(value) -> str:
    return hashlib.sha256(canonical(value).encode("utf-8")).hexdigest()


def _write_new(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise ValueError(f"refusing to overwrite {path}")
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def make_plan(cases=None) -> dict:
    selected = list(CASES if cases is None else cases)
    if not selected:
        raise ValueError("at least one context A/B case is required")
    body = {
        "schema": SCHEMA,
        "protocol": "same-model-full-context-vs-momo-assets-after-12-rounds-mixed-roleplay",
        "model": MODEL,
        "revision": REVISION,
        "rounds": 12,
        "suite_counts": {
            suite: sum(case["suite"] == suite for case in selected)
            for suite in ("morp", "acgn")
        },
        "candidate_calls": len(selected) * 2,
        "maintenance_calls": len(selected) * 2,
        "endpoints": {"gateway": "http://127.0.0.1:8788/v1", "core": "http://127.0.0.1:8765/v1"},
        "credentials": {"gateway": "MOMO_GATEWAY_API_KEY"},
        "pricing": {
            "qwen_cny_per_million": {"input": 0.8, "output": 2.7},
            "bge_m3_usd_per_million": {"input": 0.01},
            "price_observed_on": "2026-09-10",
        },
        "thresholds": {
            "absolute_quality": 60,
            "momo_preference_percent": 50,
            # Twelve rounds are a quality stress point, not a fair context-
            # economy break-even point. Keep the token delta descriptive.
            "require_probe_input_saving": False,
            "max_break_even_reuses": 12,
        },
        "cases": selected,
        "network_executed": False,
    }
    return {**body, "plan_sha256": digest(body)}


def _request(url: str, payload=None, gateway_key: str | None = None):
    headers = {"Content-Type": "application/json"}
    if gateway_key:
        headers["Authorization"] = "Bearer " + gateway_key
    data = None if payload is None else canonical(payload).encode("utf-8")
    method = "GET" if payload is None else "POST"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    # A 12-turn maintenance batch may legitimately consume most of the
    # service's 300-second provider budget; keep the harness slightly wider.
    with urllib.request.urlopen(request, timeout=330) as response:
        return json.loads(response.read().decode("utf-8"))


def _metrics(gateway: str, key: str | None) -> dict:
    return _request(gateway.rstrip("/").removesuffix("/v1") + "/metrics", gateway_key=key).get("routes", {})


def _metric_delta(before: dict, after: dict) -> dict:
    result = {}
    for route in sorted(set(before) | set(after)):
        left, right = before.get(route, {}), after.get(route, {})
        row = {}
        for field in ("requests", "succeeded", "failed", "input_tokens", "output_tokens", "latency_ms", "retries"):
            value = int(right.get(field, 0)) - int(left.get(field, 0))
            if value:
                row[field] = value
        if row:
            result[route] = row
    return result


def _answer_text(response: dict) -> str:
    return "".join(
        block.get("text", "")
        for output in response.get("output", []) if output.get("type") == "message"
        for block in output.get("content", []) if block.get("type") == "output_text"
    ).strip()


def _direct(case: dict, gateway: str, key: str | None) -> dict:
    messages = [{"role": "system", "content": case["persona"] + "\n\n" + ROLEPLAY_DIRECTION}]
    for turn in case["turns"]:
        messages.extend(({"role": "user", "content": turn["user"]},
                         {"role": "assistant", "content": turn["assistant"]}))
    messages.append({"role": "user", "content": case["query"]})
    started = time.perf_counter()
    response = _request(gateway.rstrip("/") + "/chat/completions", {
        "model": "conversation", "messages": messages, "stream": False, "max_tokens": 512,
    }, key)
    return {"answer": response["choices"][0]["message"]["content"].strip(),
            "usage": response.get("usage", {}), "seconds": time.perf_counter() - started}


def _momo(case: dict, core: str) -> dict:
    space_id = str(uuid.uuid4())
    card = _request(core.rstrip("/") + "/characters", {
        "owner_space_id": space_id, "name": case["title"], "author_name": "MOMO Context A/B",
        "character_markdown": case["persona"], "user_markdown": "",
    })
    source = _request(core.rstrip("/") + "/conversations", {
        "space_id": space_id, "title": "12-turn source", "character_id": card["id"],
    })
    started = time.perf_counter()
    for index, turn in enumerate(case["turns"], 1):
        for role in ("user", "assistant"):
            _request(core.rstrip("/") + "/messages", {
                "space_id": space_id, "conversation_id": source["id"], "role": role,
                "content": turn[role],
            })
        _request(core.rstrip("/") + "/momo/maintenance/turns", {
            "request_id": digest([case["id"], index, space_id]), "space_id": space_id,
            "user_content": turn["user"], "assistant_content": turn["assistant"],
            "memory_enabled": True, "nsg_enabled": True,
        })
    _request(core.rstrip("/") + "/momo/maintenance/drain", {"space_id": space_id})
    ingest_seconds = time.perf_counter() - started
    probe = _request(core.rstrip("/") + "/conversations", {
        "space_id": space_id, "title": "fresh probe", "character_id": card["id"],
    })
    extension = {
        "schema": "momo.responses/1.0", "request_id": digest([case["id"], "probe", space_id]),
        "personal_space_id": space_id, "conversation_space_id": space_id,
        "conversation_id": probe["id"], "character_id": card["id"],
        "memory_sources": [{"space_id": space_id, "label": "12-turn extracted assets", "weight": 100,
                            "memory": True, "semantic_graph": True}],
        "memory_write_space_id": space_id, "mo_state": True,
    }
    probe_started = time.perf_counter()
    response = _request(core.rstrip("/") + "/momo/responses", {
        "model": MODEL, "input": case["query"], "instructions": ROLEPLAY_DIRECTION,
        "max_output_tokens": 512, "context_window": 8192, "momo": extension,
    })
    if response.get("status") != "completed":
        raise ValueError("MOMO response incomplete")
    return {"answer": _answer_text(response), "usage": response.get("usage", {}),
            "probe_seconds": time.perf_counter() - probe_started, "ingest_seconds": ingest_seconds,
            "audit": response.get("momo", {})}


def run(plan: dict) -> dict:
    unsigned = {key: value for key, value in plan.items() if key != "plan_sha256"}
    if plan.get("schema") != SCHEMA or digest(unsigned) != plan.get("plan_sha256"):
        raise ValueError("invalid or modified plan")
    key = os.environ.get(plan["credentials"]["gateway"])
    gateway, core = plan["endpoints"]["gateway"], plan["endpoints"]["core"]
    rows = []
    for case in plan["cases"]:
        before = _metrics(gateway, key)
        direct = _direct(case, gateway, key)
        middle = _metrics(gateway, key)
        momo = _momo(case, core)
        after = _metrics(gateway, key)
        direct["route_metrics"] = _metric_delta(before, middle)
        momo["route_metrics"] = _metric_delta(middle, after)
        rows.append({"case_id": case["id"], "suite": case["suite"], "title": case["title"],
                     "dimension": case["dimension"],
                     "persona": case["persona"], "query": case["query"], "rubric": case["rubric"],
                     "direct_context": direct, "momo_12_turn": momo})
    body = {"schema": "momo.context-ab-results/1", "plan_sha256": plan["plan_sha256"],
            "model": plan["model"], "revision": plan["revision"], "pricing": plan["pricing"],
            "thresholds": plan["thresholds"], "cases": rows, "network_executed": True}
    return {**body, "results_sha256": digest(body)}


def blind_bundle(results: dict) -> dict:
    pairs = []
    for row in results["cases"]:
        rng = random.Random(int(hashlib.sha256(row["case_id"].encode()).hexdigest()[:16], 16))
        momo_is_a = bool(rng.getrandbits(1))
        a = row["momo_12_turn"] if momo_is_a else row["direct_context"]
        b = row["direct_context"] if momo_is_a else row["momo_12_turn"]
        pairs.append({"case_id": row["case_id"], "suite": row["suite"], "title": row["title"],
                      "dimension": row["dimension"],
                      "background": row["persona"], "query": row["query"], "rubric": row["rubric"],
                      "a": a["answer"], "b": b["answer"],
                      "answer_key": {"a": "momo_12_turn" if momo_is_a else "direct_context",
                                     "b": "direct_context" if momo_is_a else "momo_12_turn"}})
    grouped = {suite: [pair for pair in pairs if pair["suite"] == suite]
               for suite in ("morp", "acgn")}
    pairs = [pair for index in range(max(map(len, grouped.values())))
             for suite in ("morp", "acgn") if index < len(grouped[suite])
             for pair in (grouped[suite][index],)]
    direct_in = sum(int(r["direct_context"]["usage"].get("prompt_tokens", 0)) for r in results["cases"])
    momo_in = sum(int(r["momo_12_turn"]["usage"].get("input_tokens", 0)) for r in results["cases"])
    return {"schema": "momo.context-ab-review/1", "results_sha256": results["results_sha256"],
            "model": results["model"], "thresholds": results["thresholds"], "pairs": pairs,
            "observed": {"direct_probe_input_tokens": direct_in, "momo_probe_input_tokens": momo_in,
                         "probe_input_saving_percent": round((1 - momo_in / direct_in) * 100, 2) if direct_in else None},
            "review": {"reviewer": "", "choices": [], "completed_at": None}}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    planned = sub.add_parser("plan")
    planned.add_argument("--out", required=True, type=Path)
    planned.add_argument("--case-id", action="append", default=[])
    executed = sub.add_parser("run")
    executed.add_argument("--plan", required=True, type=Path)
    executed.add_argument("--out", required=True, type=Path)
    executed.add_argument("--allow-ai", action="store_true")
    review = sub.add_parser("review-bundle")
    review.add_argument("--results", required=True, type=Path)
    review.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    if args.command == "plan":
        selected = CASES
        if args.case_id:
            requested = set(args.case_id)
            selected = [case for case in CASES if case["id"] in requested]
            missing = requested - {case["id"] for case in selected}
            if missing:
                raise ValueError(f"unknown context A/B case ids: {sorted(missing)}")
        value = make_plan(selected)
    elif args.command == "run":
        if not args.allow_ai:
            raise ValueError("AI disabled; pass --allow-ai to execute the frozen plan")
        value = run(json.loads(args.plan.read_text(encoding="utf-8")))
    else:
        value = blind_bundle(json.loads(args.results.read_text(encoding="utf-8")))
    _write_new(args.out, value)
    print(canonical({"schema": value["schema"], "out": str(args.out)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
