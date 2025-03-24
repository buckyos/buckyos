一个IT系统里总是存在3种被人依靠的关键基础设施：崩溃上报（发现问题），运行统计（健康检查），自动升级（解决问题）。在一个完善的生产级系统里，总是会以不同的形态出现上述 3 个子系统。这 3 个子系统本身和系统运行的稳定性关系很大，从历史经验来看，其实现从来都不是简单的，都需要根据实际情况进行精巧的构造，保持简单，以确`用来解决问题的系统自己不会出问题`

本文档重点描述了系统如何基于业务无关的pkg-system,实现buckyos的应用安装和升级。主要涉及到下面 3 个组件
- pkg-env:
- node-daemon:
- repo-server:


## 安装 App/内核服务的全流程：
0. repo自动/手工 从源同步 pkg-index-db.
1. repo根据已安装app/service列表准备app.
准备app(pkg): 根据app-meta信息，repo下载所有的subpkg和chunk。下载完成后，该appid(pkg)在本地是ready的（所有的sub_pkg都ready了）
2. 使用appdoc产生app_config,这一步通常需要UI参与（app_setting.ts 里有手输app_doc后产出app_config的逻辑）
3. 将app_config发送给调度器，由调度器去构造具体的node_config
4. node_daemon根据node_config,会在现在本地的bin_env或app_img_env里install对应的pkg,准备好后执行deploy脚本，对app来说，这个install脚本通常是将导入docker image的tar文件。


## 发布流程
1. pack pkg，得到pkg_meta + pkg_chunkid（如有）。对于有多个sub_pkg的组件，应该先pack sub_pkg
2. publish pkg_meta to current zone: 发布后支持：
- http://repo.buckyos.org/pkg-index-db 中能通过pkgid或正确的版本查询到pkg_meta信息
- 通过http://ndn.buckyos.org/repo/pkg_id 可以得到该pkg_meta,pkg_chunk_i
- 通过http://ndn.buckyos.org/$chunkid 可以下载chunk
3. 将pkgid<->pkg_meta信息提交给源服务器。源服务器在正式收录到自己的pkg-index-db前，会确保该 pkg在源的repo-server上是ready的。
4. 任意repo-server都可以安上述流程成为一个源服务器，源服务器如何收录pkg的逻辑，源的使用者并不关心
5. 考虑到pkg-id的全局唯一性，我们在发布流程里要有一个通用服务，来解决冲突问题（注册 pkg-id <-> author),我们先用一个中心化的服务来做，几个版本后可以切换到 BNS 合约上。

## pkg_id设计
完整的pkgid （注意pkgid与named_obj_id是不同构的）
    准确id: pkg_name#pkg_meta_objid，现在的objid是chunkid,这是不对的
    版本id: pkg_name#version （version里带有 *，>=, <= 这种组合字符)
    准确版本id:pkg_name#version (version里不带有 *，>=, <= 这种组合字符)

version到pkg_meta_objid的转换需要依赖某个版本的pkg-index-db
    pkg-index-db有所在channel的概念
    env也有所在channel的概念

pkg_name完整的由2部分组成:perfix.fullname,其中 prefix的典型逻辑是nightly-linux-x86_64, fullname的典型逻辑是 $authorname-pkgname.在perfix和fullname中都不能包含 `.`, 使用`-`来连接不同含义.

在编码的时候，使用不带prefix的pkg_name来加载pkg，比如要加载nightly-apple-x86_64.fogworks-filestation 这个pkg,应该正确的写成 env.load("fogworks-filestation"),由env来正确的产生前缀。由一些本身就是全平台的包，可以一个包在所有平台下运行，其加载可以写成 env.load("all.fogworks-filestation"),一旦load的时候pkg_name包含.,env就不会启用perfix的自动附着机制。

通常源本身发布的pkg可以没有$authorname.pkg_name需要做到全网唯一，因此不同的源在收录的时候会要求$authorname（第一个`-`以前）要预先注册。但对pkg-system的基础系统来说，并不需要了解这个规则。


### env.load(pkg_id):
重要：env的大部分函数，都是只读和幂等的。这意味着在env没有改变（安装新的pkg)的情况下，执行同样的操作必然会得到相同的结果。为了开发方便，env支持在没有index-db的时候，用纯路径查找的方法完成加载。

1. pkg_id只有一个pkg_name（我们最常用的情况）
该流程总是加载 env/pkg_name/ 目录，pkg_env的安装流程会建立正确的符号链接，让其指向正确的默认版本。
2. pkg_id包含非准确版本信息
方案一、遍历 env 中存在的pkg_name的不同版本的pkg,然后选择符合条件的那个
方案二、如果env包含pkg-index-db，则通过pkg-index-db先找到正确的版本列表，再判断这些版本里哪个是env里存在的（符合条件的情况下，总是选择最新版本）
3. pkg_id包含准确的版本信息
展开到 env/pkg_name#version/ 目录，pkg_env的安装流程会建立正确的符号链接，让其指向正确的默认版本。
4. pkg_id包含准确的objid信息
展开到env/.pkgs/.pkg_name/pkg_name#objid/ 目录，这通常是一个实体目录

注意 env.load与env.try_load的区别,env.try_load会无视本地的pkg按照情况，只通过pkg-index-db来获得pkg-meta

### env.get_pkg_meta(pkg_id);
注意要与env.load保持一致，要返env.load(pkg_id)所加载pkg的meta信息。因此实现有 2 种
1. 优先查找路径，比如env/.pkgs/.pkg_name/pkg_name#objid.meta 文件存在，则优先返回该文件的内容
2. 在env.pkg-index-db中查找得到pkg-meta


### 理解env.install流程 （pkg_name的展开逻辑）：
0. env.install(pkg_name)
1. 首先查询 env.pkg-index-db,得到默认的Author; 从实现减少磁盘占用，更加简单的角度，一般一个机器上只有一个env真正拥有pkg-index-db,其他env都是通过正确的继承配置来间接的查询该pkg-index-db.
    展开得到 Author/pkg_name
    如果有两个author都发布了相同名字的pkg,那么要加载后发布的pkg必须使用用 Author/pkg_name
2. 根据当前env的channel，加载正确的pkg-index-db
    在pkg-index-db中，查询Author/pkg_name的默认版本
    如果install的时候带了version，那么在pkg-index-db中查找符合条件的version
3. 得到最终的author/pkg_name#objid
4. 实际执行的是install(uthor/pkg_name#objid)
5. 根据pkb-obj-id,得到并校验pkg-meta
6. 根据pkg-meta在下载并复制相关文件到env的约定路径，并根据env的实现细节建立符号链接。注意安装操作一定不会有删除动作，当无法安规则完成文件操作或符号建立动作时（最常见的时env/pkg_name 目录存在实体文件夹），该安装操作会失败，需要用户手动干预才会执行成功（或有特殊的控制参数）
7. pkg-meta中有保存pkg的依赖，应先安装依赖。安装成功后，可以env.load(dep_pkg)成功

## zone内升级的完整流程（repo-server已经运行起来的常规更新）
1. 系统配置成 自动升级 / 检查但需要用户确认 / 手动触发
2. repo-server定期检查上游源是否有新版，并根据上述设置对更新进行触发
3. 触发更新：
    repo-server从上游源同步pkg-index-db
    repo-server:根据新版本的pkg-index-db,尝试更新已安装的（或指定的）app/service。让新版本的app/service在 repo-server上就绪。
    repo-server:Apply更新：这里要根据具体产品需求，决定是否需要有必要更新system_config 
    调度器：如果system-config更新了其关注的项目，则触发调度修改node-config
4. 执行更新
    node-daemon:根据配置 保持bin_env/app_env 里的pkg-index-db与repo-server里的zone pkg-index-db一致
    node-daemon:当pkg-index-db改变后，node-daemon会发现一些需要deploy的instance(旧的部署已经不符合条件了)
    node-damoen:完成app新版本的部署后会启动新版本，其启动脚本里会结束上一个版本的相关服务 

## node_daemon对pkg升级的基本逻辑和潜在保障：
- 每个service_pkg的版本更新都是独立的，如果存在依赖，那么意味着在该server_pkg能成功load的时候，也已经把本地的其它包安装好了。通过依赖关系来实现原子特性要比手工整理应用相关的pkg+sub_pkg机制更可靠。
- 更新的4要素:检查有没有更新+下载更新（网络部分）， 新版本：安装+运行， 旧版本：停止+删除，自动回滚：通过默认机制，可以实现简单可靠的更新失败回滚。 自动回滚的实现思路：a.启动新版本的时候用“临时路径”启动，新版本启动完成的最后一步是建立链接，如果没执行到最后一步（比如中途重启了，那么下一次启动会回到老版本） b. 系统不正常的时候，可以通过所谓的安全模式让系统回滚到一个绝对可用的情况？或则当系统坏了的时候，用户进行手动修复的方法是什么？比如系统激活后，启动一个及其纯粹的修复服务，通过该服务可以做一切操作。通过推演可知，我们的系统不需要实现回滚。
- 只有node_daemon的self更新可能是基于zone外检查的，因为node_daemon的版本不对可能会导致无法正确启动zone 
- 一旦repo-server能正常工作,ndoe_daemon也使用标准的流程进行自我更新。

### zone外更新（或则叫启动更新）的流程，该流程中repo-service没运行起来
1. node_daemon进入boot阶段前检查或active_server阶段前检查是否存在更新
2. 如在device_config中配置默认源，则向默认源查找（标准http查找）
3. 下载完整的安装包，并执行标准安装动作（会尝试更新整个$BUCKYOS_ROOT 目录）
4. 执行标准安装动作的时候要注意 关闭node-daemon(阻止启动)，执行完成后按相同参数启动node_daemon
5. buckyos的设计目标是尽可能不修改node-daemon

## 执行升级的关键问题：node_daemon从逻辑上存在两种升级触发方法
1. 通过node_config触发，node_config里包含了instance 的完整pkg_id（包含准确的版本号或objid)，当升级时，调度起会修改pkg_id会变化（这种方法不区分升级和降级，是要求运行目标版本）。支持该方法需要所有servier_pkg的status方法能准确的判断当前版本是否在运行。并能在启动脚本中正确结束可能存在的，其他版本的正在运行的实例。

2. 通过pkg latest check实现.ndoe_config里只包含了非精确版本号的pkg_id,因此node_daemon需要判断zone内的pkg_id对应的latest版本是否改变了，并进入转换到新版本上的环节。考虑到逻辑的一致性，该检查的强一直实现也应该是先检查是否需要同步pkg-index-db，同步成功后再基于完整的本地pkg-index-db进行查询，而不是简单的通过zone-repo-services来查询pkg的latest版本是否改变（这会带来两次查询的事务问题）

结论：node_daemon的核心循环实现，应通过正确的管理bin_env/app_env更新，通过简单可靠的基础逻辑同时支持上述两种模式。后续使用那种模式是调度器需要考虑的。因此node_daemon对基于pkg-system的app安装/升级的的核心逻辑如下（重要：但看代码很难理解其完整目的）
- node_daemon中有代码，定期的从repo-server同步pkg-index-db到 env
- pkg-index-db更新后，env.try_load(pkg_id) 会返回到新版本的 pkg
- 该（新版本的）pkg在本地未安装，因此判断其状态会得到NotExist,进而触发delpoy操作
- deploy操作有两段，第一段是基于pkg-metea在env中下载安装，第二段是执行pkg类型相关的deploy脚本（比如把 app docker导入）
- deploy成功后，调用对应的start脚本，该脚本的实现里，会结束旧版本的instance，并启动新的进程

### 理解node_daemon的核心职责：保障instance以正确的状态存在于当前node上
```python
def node_main_loop()
    node_env.check_and_update_index_db(zone_repo_url)
    check_and_update_system_services() #检查系统组件的更新，检查逻辑与下面检查 item_instance的逻辑基本相同

    for item_instance in node_config.instances:
        pkg = node_env.try_load(item_instance.pkg_id)
        if is_not_exist(pkg):
            download_and_install(pkg,zone_repo_url)

        if pkg.status() != item_instance.target # 该函数的实现做非常严格的判断，确保是在检查当前版本的
            control_item(pkg,item_instance.target) # 该函数的实现里，通常包含了对旧版本的停止工作（单版本实例保障）
```


## 一些进一步的思考过程（任何分布式系统的设计，都可以例行的从下面这些角度进行更深度的思考）
### node_daemon的意图是：
- 保障node_config里描述的instance都在正确的状态
    除了运行状态，也可以用 版本删除法确保旧版不在（Node Dameon总是先处理停止，再处理启动）： 这个想法被放弃了，
- 保障本机只有node_config里描述的instance? (这个需求通常是不正确的，从运维的角度上看，这种保障会极大的效率异常情况下能用的手段)
- 如何进行本地的垃圾回收？删除bin目录下的旧版本？
  需要node_daemon在合适的时机，调用env的gc函数，实现删除。gc的逻辑的实现对env是有影响的

### 潜在的事务性（或数据完整性）保障：
将系统的升级分成了3个阶段来分别保障事务性
阶段一、repo-server通过订阅机制获得pkg-index-db更新
我们把pkg-db当成一个可验证的同步整体，当repo-server需要更新时，会将remote的整个pkg-db都同步到本地（而不是只请求本zone已经安装的应用），并确保在使用前进行了完整的校验。随后在本地进行只读的查询操作，不会出现查询版本的请求Q1，Q2之间，pkg-db发生了改变的问题。

阶段二、确保目标版本是ready的
repo-server会根据本zone已经安装的applist,去查询是否存在app-meta更新。存在更新后，会根据配置进入准备过程。准备过程就是把appmeta里引用的subpkg和依赖的pkg（这些pkg的pkg-meta必然已经存在在了当前pkg-index-db中，这个保障是由pkg发布流程保证的）都下载到本地。（因为无法确定后续添加的node的体系结构，因此会下载所有的sub_pkg,后期优化可以配置，屏蔽某些体系结构的subpkg)。当app依赖的所有pkg和chunk都下载到本zone后，我们称该app ready了。app ready以为则zone内的服务总是可以zero-depend的使用到本版本（哪怕是断网）。比如在ready后internet失效，此时启动一个新的node,该node依旧可以按标准流程完成

阶段三、执行和业务相关的安装/更新逻辑
这些逻辑的处理流程通常是 1.写入必要信息到system_config,该步骤支持事务 -> 2.调度器根据新的配置，执行调度并将调度结果写入system_config 。 上述流程如需要用户干预或确认，应该在第一步之前进行。

阶段四、各个NODE执行更新
node_daemon的核心职责是 `保障node_config里的instance都在正确的状态`，因此当上一步完成后。node_config改变。node_daemon会努力确保node-config里描述的事实发生（运行指定版本的服务）。只要node_config确定，node_daemon就会一直尝试达成上述目标，在实现目标的过程中，只依赖zone内的repo-server。

我们的系统要保障zone内一致性：当我们在node_config里配置了某个app的版本为 > 1.2时（相同instance),那么node_daemon的机制会保障：在zone内的不同node上都一定会最终定位到相同的版本。


### 是否存在潜在的外部依赖和假设
1. 外部源失效：如果repo-server已经完成了首次同步（或则安装包里默认带有一个版本的pkg-index-db),则可有效应对
2. 假设外部源是善良的：信任源收录的pkg-meta是最新版本的，但也不用担心源有能力伪造pkg-meta。这里的潜在假设是相同pkg-id的author是不会改变的，当通过源更新后，发现已有pkg-meta的author-id修改，应视作高风险行为发生，需要用户干预后才会应用新版本的pkg-index-db
3. 假设外部源是守序的，即通过就版本安装成功的pkg-id<->pkg-meta总是存在于新版本的pkg-index-db中，并且收录的pkg的所依赖的pkgs也都必定保存在pkg-index-db中：当已经安装的pkg在pkg-index-db更新后，出现dep-not-found的情况，需要用户干预才会使用新版本的pkg-index-db


### 是否存在“只增不减问题”
1. repo-server上保存的pkg如何删除？ 
    通过本机已安装的app-list/service-list，标注必须存在pkg,随后把未标记的chunk删除
    如果repo-server还需要保存自己发布的pkg,那么就需要把所有的，作者是特定用户的pkg也标记为永久保存
    ndn_data_mgt提供的路径模式提供了一种通用思路：如果一个named-object需要保存，那么就必须给讲其关联到一个逻辑路径上。一个路径路径只能指向一个objid,因此在相同的逻辑路径上更新obj-id时，旧的named-object就有机会被named_mgr的gc删除掉。
2. pkg-index-db必须是增量的？即在旧版本里存在的pkg，在版本里也一定存在，这会导致pkg-index-db实际上会越来越大，最后同步成本越来越高。
3. node-daemon下载到本地的pkg如何删除？
- env提供gc函数，其实现必然会要求某种有效性标记。需要在env里维护一个“已本地安装的pkg列表”，以此为起点，对pkg依赖的pkg进行染色，所以未被染色的版本都会被标注为可 GC。
- env提供卸载函数,标准某个版本不再需要了（注意所谓的标注逻辑，意味着有人标注需要，有人标注为不需要，最终结果是需要）。
- 另一个简单的思路：简单的全部删除，然后依赖node-daemon的自动修复尝试，自动完成当前版本的重建。


### 自我更新（或则循环依赖问题）
node_daemon保障了所有的app/service都在本node上以正确的版本运行，那么node_daemon如何更新到指定版本？

node_daemon在boot成功后（定义为能连上zone内的system config),立刻进行一次版本校验，确保自己的版本与zone内的版本是一致的.随后node_daemon会在main-loop函数中尝试准备自己的正确版本，并在确认完成了版本改变后（不管是升级还是降级），结束自己。然后等待systemd的保活逻辑 ： 拉起start_node_daemon.py 该脚本会通过原子性的重名名操作，将node_daemon更新到新版本（需求：node_daemon需要通过编译器的内置函数，而不是查询env来获得当前版本，开发环境下的特殊版本永远不会触发自升级）


## 开发/运维的便利性支持

### 开发模式（开发者从通过git获得的版本)

- 开发者模式的ENV通常没有.pkgs目录，都是通过实际的pkg-name目录加载
- 开发者模式依旧可以设置ENV的父ENV，父ENV可以是非开发者模式的
- 默认是不会启用自动升级的，但可以使用app安装。防止自动升级覆盖了用户正在开发的组件
- 开发模式可以手工激活自动升级
- 开发模式可以手工进行升级（会有风险提示）

## 校验总是发生在从zone外获取信息时，zone内获取信息不做来源校验，本地文件不做校验，方便开发期间修改后立刻生效
## 支持开发文件夹，更新后立刻看到效果
## 支持.toml格式的pkg-index-db
## 通用.lock文件让默认版本与pkg-index-db中的配置不同


-----------------------------
index-db的设计
给定pkg-name，枚举所有存在的版本号

通过metobj-id得到metaobj的行为是标准的named-object行为，应该记录在另一个表

表一 pkg_metas
metaobjid,pkg-meta,author,author-pk,update_time, 

表二 pkg_versions
pkgname, version, metaobjid, tag, update_time
pkgname-version 形成了唯一的key

表三 author_info
author_name, author_owner_config,author_zone_config


查询接口
get_pkg_meta(pkg_name,author,version),version不填表示最新版本
get_author_info(author_name)
get_all_pkg_versions(pkg_name)

修改接口
add_pkg_meta(metaobjid,pkg-meta,author,author-pk) 
set_pkg_version(pkgname,version,metaobjid)
set_author_info(author_name,author_owner_config,author_zone_config)

pkg-env的目录结构设计
在work-dir下有一系列json格式的配置文件
.env.config 


#根据pkg_id加载已经成功安装的pkg
def env.load(pkg_id):
    meta_db = get_meta_db()
    if meta_db
        pkg_meta = meta_db.get_pkg_meta(pkg_id)
        if pkg_meta:
            pkg_strict_dir = get_pkg_strict_dir(pkg_meta)
            if os.exist(pkg_strict_dir)
                return PkgMediaInfo(pkg_strict_dir)
        
    if not self.strict_mode:
        pkg_dir = get_pkg_dir(pkg_id)
        if pkg_dir:
            pkg_meta_file = pkg_dir.append(".pkg.meta")
            local_meta = load_meta_file(pkg_meta_file)
            if local_meta:
                if staticfy(local_meta.version,pkg_id):
                    return PkgMediaInfo(pkg_dir)
            else:
                return return PkgMediaInfo(pkg_dir)


#根据pkg_id加载pkg_meta
def env.get_pkg_meta(pkg_id):
    if self.lock_db:
        lock_meta = self.lock_db.get(pkg_id)
        if lock_meta:
            return lock_meta

    if meta_db
        pkg_meta = meta_db.get_pkg_meta(pkg_id)
        if pkg_meta:
            pkg_strict_dir = get_pkg_strict_dir(pkg_meta)
            if os.exist(pkg_strict_dir)
                return PkgMediaInfo(pkg_strict_dir)

#根据pkg_id判断是否已经成功安装，注意会对deps进行检查
def env.check_pkg_ready(pkg_id,need_check_deps):
    pkg_meta = get_pkg_meta(pkg_id)
    if pkg_meta is none:
        return false

    if pkg_meta.chunk_id:
        if not ndn_mgr.is_chunk_exist(pkg_meta.chunk_id):
            return false;

    if need_check_deps:
        deps = env.cacl_pkg_deps(pkg_meta)
        for pkg_id in deps:
            if not check_pkg_ready(pkg_id,false):
                return false
        
        return true
# 在env中安装pkg
def env.install_pkg(pkg_id,install_deps)
    env.lock_for_write() #注意这是一个写操作，要做基于文件系统的全局锁

    if self.ready_only
        return err("READ_ONLY")

    pkg_meta = get_pkg_meta(pkg_id)
    if pkg_meta is none:
        return err("unknown pkg_id")
    if install_deps:
        deps = env.cacl_pkg_deps(pkg_meta)

    //有一个消费者线程专门处理单个pkg的安装
    task_id,is_new_task = env.install_task.insert(pkg_id)
    if is_new_task && install_deps:
        for pkg_id in deps:
            env.install_task.insert_sub_task(pkg_id,task_id)
        
    return task_id,is_new_task

# 内部函数，从install_task队列中提取任务执行
def env.install_worker():
    let install_task = env.install_task.pop()
    #下载到env配置的临时目录，不配置则下载到ndn_mgr的统一chunk目录
    download_result = download_chunk(install_task.pkg_meta.chunkid)
    if download_result:
        pkg_strict_dir = get_pkg_strict_dir(pkg_meta)
        unzip(download_result.fullpath,pkg_strict_dir)
        if self.enable_link:
            create_link(install_task.pkg_meta)
        notify_task_done(install_task)

# 异步编程支持,可以等待一个task的结束
def env.wait_task(taskid)

# 尝试更新env的meta-index-db,传入的new_index_db是新的index_db的本地路径
def env.try_update_index_db(new_index_db):
    #当有安装任务存在时，无法更新index_db
    env.try_lock_for_write()
    #重命名当前文件
    rename_file(index_db,index_db.append(".old"))
    #移动新文件到当前目录
    rename_file(new_index_db,index_db)
    #删除旧版本的数据库文件
    delete_file(index_db.append(".old"))








    

        




