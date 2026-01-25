## 开发环境与pkg的更新

通过git clone得到的开发环境默认是一个独立环境，未设置source-service

### 最简单的单机环境

用现有脚本完成构建后立刻运行，不需要使用任何pkg-system的设施来发布/更新

### 使用有身份的自编译环境(nightly-channel,使用真实身份）

核心是与nightly-SN的版本保持同步，应使用nightly环境下SN兼容的版本进行开发测试
git checkout到正确的版本，然后编译运行
该环境下可以发布 app(流程与release环境相同)

## 标准的应用开发环境

确认repo-server已配置成开发模式（git clone的版本默认就是开发环境）
构建时要注意app sub pkg-name必须是完整版的：nightly-apple-x86_64.$pkg_name#version

### 构建 （TODO：app 标准开发目录，考虑到全平台编译工具的完善，默认使用linux，windows和osx平台需要用户手工build好复制到target目录)

1. 构建全平台2进制 --> 构建对应的Docker
2. 基于docker image 构建全平台的image sub_pkg
3. 如果是系统应用，还需要基于二进制再构建全平台的bin sub_pkg

为了提高效率，在测试阶段可以只针对开发环境构建一个平台的pkg

### 测试

1. 注意更新app_name.app_doc.json 的版本
2. 使用buckycli pub_app 将app发布到zone内
3. 使用buckycli pub_index 使发布在zone内生效
   4.1 使用buckycli 在zone内安装应用（如已安装可逃过）
   4.2 zone内已经运行的app会变成新版本

### 发布

根据不同的source-service的规则，提交新的app_doc.
发布的时候有2个channel (nightly / release),这两个环境是独立的。 不依赖nightly新功能的应用开发者，只需要基于release channel进行构建和发布就好了。
在提交app_doc之前，应确保已经获得了 可提交资格（已认证开发者)

## buckyos系统的构建与发布

我们构建的发布目标有

- deb/rpm 全新安装包 (最好是一个下载安装脚本，总是可以在最新版上)

  - 该最新版的URL也可以支持一些出厂硬件在首次激活前的自动更新
  - 有两个版本的安装包，一个是标准的在线安装包，只包含必要的基础服务，其它服务都是通过repo升级机制从source service上下载的。另一个是完整版安装包，安装成功后已经相当于更新到了最新版本
  - 完整版安装包通常会比在线安装包的版本更新（虽然时间不长）
- buckyos desktop 安装包
- 将buckyos的pkg_list发布到source-service上，当前channel且启用了自动更新的运行中系统，会很快更新到新版本上

### 日常开发循环

常规逻辑可以使用最简单的单机环境进行开发和验证，可以通过删除 $BUCKYOS_ROOT/data/system_config 让系统从boot流程开始。该开发循环用到的所有pkg-env都处于开发模式，不会启用自动升级
但手工向该repo-server发布pkg是可行的,发布的pkg会按正常逻辑影响zone内的所有node

完成基本的开发测试后，要进行进一步验证。为了防止受到外界环境的影响，buckyos的系统开发验证过程一定使用的是独立环境。我们提供了一些简洁的虚拟机模板，用来模拟一些典型的集群环境（这些集群环境里包含刚刚构建的独立SN），并有基于这些典型集群环境的测试用例。

系统开发者可以基于这些典型环境设计用例，或进行开发。当需要在这些典型环境中部署刚刚编译的软件时，就需要手工向zone repo 发布pkg:

- 运行脚本，会准备待pack的目录，可以进行一些打包前的检查
- 继续运行脚本，完成所有的pkg的pack
- 执行发布脚本，发布选中的packed pkg到本地repo，并调用pub index让其立刻生效

### buckyos nightly build

系统开发者完成日常开发循环后，提交代码。合并到buckyso 当前nightly分支的代码会触发自动构建。自动构建按如下步骤进行（用户也可在本地环境手工触发自动构建）

- 完成所有支持平台的二进制编译和root fs准备
- 基于构建结果构建deb/rpm安装包
- 基于构建结果构建buckyos desktop安装包
- 基于构建结果构建全平台的pkg
- 按流程启动典型环境（OOD一般是linux的虚拟机），并部署刚构建的二进制包
- 在典型环境中运行所有的测试用例
- 验证通过后，本次CI构建完成，开始发布
  - 创建新的github nightly release
  - 将安装包发布到nightly的下载服务器（用脚本进行安装会得到新版本）
  - 将pkg推送到nightly的source-service,所有运行在nightly环境系统都会触发自动升级

#### always run

我们有多个运行在nightly 环境的测试机，会从用户的角度，不断的模拟一些常见动作。

- 各个支持平台的自动安装
- 打开自动更新的模拟生产系统，不断的运行一些always run测试用例

在nightly环境发布后，上述测试如果失败会给出自动警告，说明该版本有问题。手工验证后可以撤销某个nightl build.

从维护成本的角度，我们不实现真正意义上的“回滚”，而是通过发布一个修正了问题的新的nightly版本来实现同样的效果

### 正式发布

正式发布通常都是选择发布一个指定的nightly版本
选择需要正式发布的nightly版本后，相关脚本可以自动化的运行并完成发布工作。

- 创建新的github release
- 将安装包发布到release的下载服务器（用脚本进行安装会得到新版本）
- 将pkg推送到release的source-service,所有运行在nightly环境系统都会触发自动升级

我们在release环境也有always run用于实时监控release版本的系统是否在正常工作。
